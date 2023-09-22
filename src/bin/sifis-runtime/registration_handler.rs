use std::{borrow::Cow, sync::Arc};

use anyhow::{anyhow, Context};
use log::{info, trace, warn};
use serde::Deserialize;
use sifis_dht::domocache::DomoCache;
use sifis_message::LAMP_TOPIC_NAME;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{
    continuation, domo_wot_bridge,
    registration::{self, Registration, ThingTypeRegistration},
    ucs, AccessRequest, AccessRequestKind, PeerId, QueuedResponseMessage,
};

pub(super) async fn handle_registration_message(
    message: Registration,
    domo_cache: &mut DomoCache,
    client_id: Arc<str>,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
    response_sender: mpsc::Sender<QueuedResponseMessage>,
) -> anyhow::Result<()> {
    let helper = Helper {
        domo_cache,
        client_id,
        ucs_topic_name,
        ucs_topic_uuid,
        response_sender,
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

        Registration::GetTopicName { name, responder } => {
            let response = helper.domo_cache.get_topic_name(&name);
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
    domo_cache: &'a mut DomoCache,
    client_id: Arc<str>,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
    response_sender: mpsc::Sender<QueuedResponseMessage>,
}

impl Helper<'_> {
    async fn register_to_ucs(self, topic_name: Arc<str>, topic_uuid: Uuid) -> anyhow::Result<()> {
        let Self {
            domo_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
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
        domo_cache
            .pub_value(
                serde_json::to_value(request)
                    .context("unable to convert pep command request to JSON")?,
            )
            .await;
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
            domo_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
        } = self;

        match operation {
            registration::ThingOperation::RequestPermission(peer_id) => {
                let access_request = create_access_request(
                    &thing_type.thing_uuid,
                    &thing_type.registration,
                    peer_id,
                );
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

                domo_cache
                    .pub_value(
                        serde_json::to_value(message)
                            .context("unable to convert pep command request to JSON")?,
                    )
                    .await;
            }

            registration::ThingOperation::Interact => {
                match thing_type.registration {
                    ThingTypeRegistration::Lamp(lamp) => match lamp {
                        registration::Lamp::OnOff(responder) => {
                            let result = domo_cache
                                .get_topic_uuid(LAMP_TOPIC_NAME, &thing_type.thing_uuid)
                                .map_err(|_| registration::ResponseError)
                                .and_then(|raw_topic| {
                                    let topic = domo_wot_bridge::Topic::<domo_wot_bridge::Lamp>::deserialize(&raw_topic)
                                            .map_err(|_| registration::ResponseError)?;
                                    Ok(topic.value.status)
                                });

                            responder
                                .send(result)
                                .map_err(|_| anyhow!("response sender channel is closed"))?;
                        }
                        registration::Lamp::SetOn { value, responder } => {
                            use domo_wot_bridge::{
                                Command, CommandInner, TurnCommand, VolatileMessage,
                            };

                            let message =
                                VolatileMessage::new(Command::TurnCommand(CommandInner {
                                    topic_name: LAMP_TOPIC_NAME,
                                    topic_uuid: &thing_type.thing_uuid,
                                    inner: TurnCommand {
                                        desired_state: value,
                                    },
                                }));

                            domo_cache
                                .pub_value(
                                    serde_json::to_value(message).context(
                                        "unable to convert turn command for lamp into JSON",
                                    )?,
                                )
                                .await;

                            response_sender.send(
                                QueuedResponseMessage::RegisterPersistentMessage {
                                    topic_name: LAMP_TOPIC_NAME,
                                    topic_uuid: thing_type.thing_uuid,
                                    responder,
                                },
                            ).await.context("unable to send registration for persistent message, channel closed")?;
                        }
                    },
                };
            }
        }

        Ok(())
    }

    async fn handle_to_ucs(self, to_ucs: continuation::ToUcs) -> anyhow::Result<()> {
        use continuation::{ToUcs, ToUcsKind};
        use ucs::AfterTryRequestPurpose;

        let Self {
            domo_cache,
            client_id,
            ucs_topic_name,
            ucs_topic_uuid,
            response_sender,
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

        domo_cache
            .pub_value(
                serde_json::to_value(message)
                    .context("unable to convert pep command request to JSON")?,
            )
            .await;

        Ok(())
    }
}

fn create_access_request(
    thing_uuid: &str,
    registration: &ThingTypeRegistration,
    peer_id: PeerId,
) -> ucs::TryAccessRequest {
    use registration::Lamp;
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
