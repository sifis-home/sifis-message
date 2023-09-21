use std::{borrow::Cow, ops::Not};

use anyhow::{bail, Context};
use log::{debug, trace, warn};
use reqwest::{
    header::{HeaderValue, CONTENT_TYPE},
    Method,
};
use sifis_message::{MessageBody, RequestMessage};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use wot_td::{
    protocol::http,
    thing::{Form, FormOperation, PropertyAffordance},
    Thing,
};

use crate::{
    registration::{self, Registration, ThingTypeRegistration},
    AccessRequestKind, QueuedResponseMessage, Responder,
};

#[derive(Debug)]
pub enum Continuation {
    Thing {
        thing: Thing<http::HttpProtocol>,
        ty: registration::ThingType,
    },
    ToUcs(ToUcs),
    Interact {
        request_uuid: Uuid,
        ty: registration::ThingType,
    },
}

#[derive(Debug)]
pub struct ToUcs {
    pub session_id: Uuid,
    pub request_uuid: Uuid,
    pub kind: ToUcsKind,
    pub thing_type: registration::ThingType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToUcsKind {
    StartResponse,
    EndResponse,
}

impl ToUcs {
    pub(super) fn access_request_kind(&self) -> AccessRequestKind {
        let &Self {
            session_id, kind, ..
        } = self;
        match kind {
            ToUcsKind::StartResponse => AccessRequestKind::Start { session_id },
            ToUcsKind::EndResponse => AccessRequestKind::End,
        }
    }
}

pub(super) async fn handle_receiver(
    mut continuation_receiver: mpsc::UnboundedReceiver<Continuation>,
    response_sender: &mpsc::Sender<QueuedResponseMessage>,
    cache_sender: &mpsc::Sender<Registration>,
) -> anyhow::Result<()> {
    while let Some(continuation) = continuation_receiver.recv().await {
        match continuation {
            Continuation::Thing { thing, ty } => {
                handle_thing(thing, ty, response_sender, cache_sender).await?;
            }

            Continuation::ToUcs(to_ucs) => cache_sender
                .send(Registration::ToUcs(to_ucs))
                .await
                .context("unable to queue message to be sent to the UCS")?,

            Continuation::Interact { request_uuid, ty } => cache_sender
                .send(Registration::Thing {
                    request_uuid,
                    ty,
                    operation: registration::ThingOperation::Interact,
                })
                .await
                .context("unable to queue message to interact with thing")?,
        }
    }

    bail!("continuation channel is closed");
}

async fn handle_thing(
    thing: Thing<http::HttpProtocol>,
    ty: registration::ThingType,
    response_sender: &mpsc::Sender<QueuedResponseMessage>,
    cache_sender: &mpsc::Sender<Registration>,
) -> anyhow::Result<()> {
    trace!("Received thing message from continuation channel. Type={ty:?}");
    todo!();

    let (responder, body) = match ty.registration {
        ThingTypeRegistration::Lamp(registration::Lamp::OnOff(sender)) => {
            (Responder::Bool(sender), None)
        }

        ThingTypeRegistration::Lamp(registration::Lamp::SetOn { responder, value }) => (
            Responder::Unit(responder),
            Some(serde_json::Value::from(value)),
        ),
    };

    let (done_sender, done_receiver) = oneshot::channel();
    let response_message = QueuedResponseMessage::Register {
        uuid: request_message.request_id,
        responder,
        done: done_sender,
    };

    debug!("Sending response message to channel: {response_message:#?}");
    response_sender
        .send(response_message)
        .await
        .context("response sender channel is closed")?;

    debug!("Waiting done event");
    done_receiver.await.context("done channel is closed")?;

    debug!("Sending registration of request message to channel: {request_message:#?}");
    cache_sender
        .send(Registration::Raw(request_message))
        .await
        .context("cache sender channel closed")?;

    Ok(())
}
