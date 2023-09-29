use std::{borrow::Cow, sync::Arc};

use anyhow::Context;
use log::{debug, info, trace, warn};
use sifis_message::RequestMessage;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    continuation::{self, Continuation},
    registration::{self, Registration, ThingTypeRegistration},
    ucs, AccessRequest, AccessRequestKind, PeerId, QueuedResponseMessage, Responder,
};

pub(super) async fn handle_registration_message(
    message: Registration,
    dht_cache: &sifis_dht::cache::Cache,
    client_id: Arc<str>,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
    response_sender: mpsc::Sender<QueuedResponseMessage>,
    continuation_sender: mpsc::UnboundedSender<Continuation>,
) -> anyhow::Result<()> {
    let helper = Helper {
        dht_cache,
        client_id,
        ucs_topic_name,
        ucs_topic_uuid,
        response_sender,
        continuation_sender,
    };

    match message {
        Registration::RegisterToUcs {
            topic_name,
            topic_uuid,
        } => helper.register_to_ucs(topic_name, topic_uuid).await,

        Registration::Thing {
            request_uuid,
            ty,
            operation,
        } => helper.handle_thing(request_uuid, ty, operation).await,

        Registration::Raw(request_message) => pub_request_message(&request_message, dht_cache),
        Registration::GetTopicName { name, responder } => {
            let query = dht_cache.query(&name);
            let query = query.get().await;
            let response = query.iter().map(|(uuid, _)| uuid.to_owned()).collect();

            info!("GetTopicName for name {name} got a response: {response:#?}");
            if responder.send(response).is_err() {
                warn!("cannot respond to get_topic_name, channel closed");
            }
            Ok(())
        }
        Registration::ToUcs(to_ucs) => helper.handle_to_ucs(to_ucs).await,
    }
}

pub struct Helper<'a> {
    dht_cache: &'a sifis_dht::cache::Cache,
    client_id: Arc<str>,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
    response_sender: mpsc::Sender<QueuedResponseMessage>,
    continuation_sender: mpsc::UnboundedSender<Continuation>,
}

impl Helper<'_> {
    async fn register_to_ucs(self, topic_name: Arc<str>, topic_uuid: Uuid) -> anyhow::Result<()> {
        let Self {
            dht_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
            continuation_sender: _,
        } = self;

        let message_id = Uuid::new_v4();
        let response_message = QueuedResponseMessage::UcsRegistration { message_id };
        response_sender
            .send(response_message)
            .await
            .context("unable to register to UCS")?;

        let request = ucs::Request::pep_command(ucs::MessageContainer {
            message: ucs::RegisterInstance {
                purpose: ucs::RegisterInstancePurposee::default(),
                message_id,
                sub_topic_name: topic_name,
                sub_topic_uuid: topic_uuid,
            },
            id: Cow::Borrowed(&client_id),
            topic_name: Cow::Borrowed(&ucs_topic_name),
            topic_uuid: Cow::Borrowed(&ucs_topic_uuid),
        });
        dht_cache
            .send(
                serde_json::to_value(request)
                    .context("unable to convert pep command request to JSON")?,
            )
            .context("unable to send request to DHT cache")?;
        trace!("Published registration message to UCS through DHT");

        Ok(())
    }

    async fn handle_thing(
        self,
        request_uuid: Uuid,
        thing_type: registration::ThingType,
        operation: registration::ThingOperation,
    ) -> anyhow::Result<()> {
        let Self {
            dht_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
            continuation_sender,
        } = self;

        match operation {
            registration::ThingOperation::RequestPermission(peer_id) => {
                let access_request =
                    create_access_request(thing_type.thing_uuid, &thing_type.registration, peer_id);
                let (responder_sender, responder_receiver) = oneshot::channel();

                response_sender
                    .send(QueuedResponseMessage::UcsAccessRequest {
                        request: AccessRequest {
                            request_uuid,
                            message_id: access_request.message_id,
                            kind: AccessRequestKind::Try,
                            thing_type,
                        },
                        responder: responder_sender,
                    })
                    .await
                    .context("unable to send access request to other task")?;

                responder_receiver
                    .await
                    .context("unable to receive notification from thing responder")?;

                let message = ucs::Request::pep_command(ucs::MessageContainer {
                    message: access_request,
                    id: Cow::Borrowed(&client_id),
                    topic_name: Cow::Borrowed(&ucs_topic_name),
                    topic_uuid: Cow::Borrowed(&ucs_topic_uuid),
                });

                dht_cache
                    .send(
                        serde_json::to_value(message)
                            .context("unable to convert pep command request to JSON")?,
                    )
                    .context("unable to send message to DHT")?;
            }

            registration::ThingOperation::Interact => {
                let request_message = RequestMessage::new(thing_type.thing_uuid, ".well-known/wot");
                let (done_sender, done_receiver) = oneshot::channel();
                let response_message = QueuedResponseMessage::Register {
                    uuid: request_message.request_id,
                    responder: Responder::Thing {
                        sender: continuation_sender.clone(),
                        registration: thing_type,
                    },
                    done: done_sender,
                };

                response_sender
                    .send(response_message)
                    .await
                    .context("response sender channel is closed")?;
                debug!("Waiting done event");
                done_receiver.await.context("done channel is closed")?;

                pub_request_message(&request_message, dht_cache)?;
            }
        }

        Ok(())
    }

    async fn handle_to_ucs(self, to_ucs: continuation::ToUcs) -> anyhow::Result<()> {
        use continuation::{ToUcs, ToUcsKind};
        use ucs::AfterTryRequestPurpose;

        let Self {
            dht_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
            continuation_sender: _,
        } = self;

        let kind = to_ucs.access_request_kind();
        let ToUcs {
            session_id,
            request_uuid,
            thing_type,
            ..
        } = to_ucs;

        let message_id = Uuid::new_v4();
        let (done_sender, done_receiver) = oneshot::channel();
        let response_message = QueuedResponseMessage::UcsAccessRequest {
            request: AccessRequest {
                request_uuid,
                message_id,
                kind,
                thing_type,
            },
            responder: done_sender,
        };
        response_sender
            .send(response_message)
            .await
            .context("unable to send access request message to the handler task")?;
        done_receiver.await?;

        let purpose = match to_ucs.kind {
            ToUcsKind::StartResponse => AfterTryRequestPurpose::Start,
            ToUcsKind::EndResponse => AfterTryRequestPurpose::End,
        };

        let message = ucs::Request::pep_command(ucs::MessageContainer {
            message: ucs::AfterTryRequest {
                purpose,
                message_id,
                session_id,
            },
            id: Cow::Borrowed(&client_id),
            topic_name: Cow::Borrowed(&ucs_topic_name),
            topic_uuid: Cow::Borrowed(&ucs_topic_uuid),
        });

        dht_cache
            .send(
                serde_json::to_value(message)
                    .context("unable to convert pep command request to JSON")?,
            )
            .context("unable to send message to DHT")?;

        Ok(())
    }
}

fn pub_request_message(
    request_message: &RequestMessage,
    dht_cache: &sifis_dht::cache::Cache,
) -> anyhow::Result<()> {
    let request_message = serde_json::to_value(request_message)
        .context("unable to convert thing description request to JSON")?;

    trace!("Publishing request message: {request_message:#?}");
    dht_cache
        .send(request_message)
        .context("unable to send message to DHT cache")?;
    Ok(())
}

fn create_access_request(
    thing_uuid: Uuid,
    registration: &ThingTypeRegistration,
    peer_id: PeerId,
) -> ucs::TryAccessRequest {
    use registration::{Lamp, Sink};
    use ucs::try_access_request::RequestKind;
    use ThingTypeRegistration as TTR;

    let kind = match *registration {
        TTR::Lamp(Lamp::OnOff(_)) => RequestKind::GetStatus,
        TTR::Lamp(Lamp::SetOn {
            value: true,
            responder: _,
        }) => RequestKind::TurnOn,
        TTR::Lamp(Lamp::SetOn {
            value: false,
            responder: _,
        }) => RequestKind::TurnOff,
        TTR::Lamp(Lamp::Brightness(_)) => RequestKind::GetBrightness,
        TTR::Lamp(Lamp::SetBrightness {
            value,
            responder: _,
        }) => RequestKind::SetBrightness(value),
        TTR::Sink(Sink::Flow(_)) => todo!(),
        TTR::Sink(Sink::Temp(_)) => todo!(),
        TTR::Sink(Sink::Level(_)) => todo!(),
        TTR::Sink(Sink::SetFlow {
            value: _,
            responder: _,
        }) => todo!(),
        TTR::Sink(Sink::SetTemp {
            value: _,
            responder: _,
        }) => todo!(),
        TTR::Sink(Sink::SetDrain {
            open: _,
            responder: _,
        }) => todo!(),
    };

    let message_id = Uuid::new_v4();
    let subject = peer_id.to_string();
    let resource = thing_uuid.to_string();

    ucs::TryAccessRequest {
        message_id,
        request: ucs::try_access_request::Request {
            subject,
            resource,
            kind,
        },
    }
}
