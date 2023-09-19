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
    let (prop_by, op) = get_prop_by_op(&ty.registration);
    let property = thing
        .properties
        .as_ref()
        .and_then(|properties| match prop_by {
            PropBy::Name(prop_name) => properties
                .iter()
                .find(|(name, _)| name.as_str() == prop_name),
            PropBy::AtType(prop_attype) => properties.iter().find(|(_, property)| {
                property
                    .interaction
                    .attype
                    .as_ref()
                    .map_or(false, |attypes| {
                        attypes.iter().any(|attype| attype == prop_attype)
                    })
            }),
        })
        .map(|(_name, property)| property);

    let Some(property) = property else {
        warn!("unable to find property {prop_by:?} on following thing description: {thing:#?}");
        return Ok(());
    };

    let Some(form) = get_request_form(property, op) else {
        warn!("unable to find form with operation {op} on property {property:#?}");
        return Ok(());
    };

    let mut request_message = RequestMessage::new(ty.thing_uuid, Cow::Owned(form.href.clone()));
    let method = get_method(form.other.method_name, op);
    request_message.set_method(method);

    let (responder, body) = match ty.registration {
        ThingTypeRegistration::Lamp(registration::Lamp::OnOff(sender)) => {
            (Responder::Bool(sender), None)
        }
        ThingTypeRegistration::Lamp(registration::Lamp::Brightness(sender))
        | ThingTypeRegistration::Sink(
            registration::Sink::Flow(sender)
            | registration::Sink::Temp(sender)
            | registration::Sink::Level(sender),
        ) => (Responder::U8(sender), None),

        ThingTypeRegistration::Lamp(registration::Lamp::SetOn { responder, value })
        | ThingTypeRegistration::Sink(registration::Sink::SetDrain {
            responder,
            open: value,
        }) => (
            Responder::Unit(responder),
            Some(serde_json::Value::from(value)),
        ),

        ThingTypeRegistration::Lamp(registration::Lamp::SetBrightness { responder, value })
        | ThingTypeRegistration::Sink(
            registration::Sink::SetFlow { responder, value }
            | registration::Sink::SetTemp { responder, value },
        ) => (Responder::Unit(responder), Some(value.into())),
    };

    if let Some(body) = body {
        request_message.body = MessageBody::String(serde_json::to_string(&body).unwrap());
        request_message
            .headers
            .push((CONTENT_TYPE, HeaderValue::from_static("application/json")));
    }

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

fn get_request_form(
    property: &PropertyAffordance<http::HttpProtocol>,
    op: FormOperation,
) -> Option<&Form<http::HttpProtocol>> {
    match op {
        FormOperation::ReadProperty => property.interaction.forms.iter().find(|form| {
            matches!(
                &form.op,
                wot_td::thing::DefaultedFormOperations::Custom(ops)
                if ops.contains(&op),
            ) || matches!(
                form.op,
                wot_td::thing::DefaultedFormOperations::Default
                if property.data_schema.write_only.not(),
            )
        }),
        FormOperation::WriteProperty => property.interaction.forms.iter().find(|form| {
            matches!(
                &form.op,
                wot_td::thing::DefaultedFormOperations::Custom(ops)
                if ops.contains(&op),
            ) || matches!(
                form.op,
                wot_td::thing::DefaultedFormOperations::Default
                if property.data_schema.read_only.not(),
            )
        }),
        _ => unimplemented!("{op} operations is still not supported"),
    }
}

#[derive(Debug, Clone, Copy)]
enum PropBy {
    AtType(&'static str),
    Name(&'static str),
}

fn get_prop_by_op(registration: &ThingTypeRegistration) -> (PropBy, FormOperation) {
    match registration {
        ThingTypeRegistration::Lamp(registration::Lamp::OnOff(_)) => {
            (PropBy::AtType("OnOffProperty"), FormOperation::ReadProperty)
        }
        ThingTypeRegistration::Lamp(registration::Lamp::Brightness(_)) => (
            PropBy::AtType("BrightnessProperty"),
            FormOperation::ReadProperty,
        ),
        ThingTypeRegistration::Lamp(registration::Lamp::SetOn { .. }) => (
            PropBy::AtType("OnOffProperty"),
            FormOperation::WriteProperty,
        ),
        ThingTypeRegistration::Lamp(registration::Lamp::SetBrightness { .. }) => (
            PropBy::AtType("BrightnessProperty"),
            FormOperation::WriteProperty,
        ),
        ThingTypeRegistration::Sink(registration::Sink::Flow(_)) => {
            (PropBy::Name("flow"), FormOperation::ReadProperty)
        }
        ThingTypeRegistration::Sink(registration::Sink::Temp(_)) => {
            (PropBy::Name("temperature"), FormOperation::ReadProperty)
        }
        ThingTypeRegistration::Sink(registration::Sink::Level(_)) => {
            (PropBy::Name("level"), FormOperation::ReadProperty)
        }
        ThingTypeRegistration::Sink(registration::Sink::SetFlow { .. }) => {
            (PropBy::Name("flow"), FormOperation::WriteProperty)
        }
        ThingTypeRegistration::Sink(registration::Sink::SetTemp { .. }) => {
            (PropBy::Name("temperature"), FormOperation::WriteProperty)
        }
        ThingTypeRegistration::Sink(registration::Sink::SetDrain { .. }) => {
            (PropBy::AtType("OnOffSwitch"), FormOperation::WriteProperty)
        }
    }
}

fn get_method(method_name: Option<http::Method>, op: FormOperation) -> Method {
    method_name.map_or_else(
        || match op {
            FormOperation::ReadProperty
            | FormOperation::ObserveProperty
            | FormOperation::QueryAction
            | FormOperation::SubscribeEvent
            | FormOperation::ReadAllProperties
            | FormOperation::ReadMultipleProperties
            | FormOperation::ObserveAllProperties
            | FormOperation::SubscribeAllEvents
            | FormOperation::QueryAllActions => Method::GET,

            FormOperation::WriteProperty
            | FormOperation::WriteAllProperties
            | FormOperation::WriteMultipleProperties => Method::PUT,

            FormOperation::UnobserveProperty
            | FormOperation::UnsubscribeEvent
            | FormOperation::UnobserveAllProperties
            | FormOperation::UnsubscribeAllEvents => {
                unimplemented!("the connection should be just terminated")
            }

            FormOperation::InvokeAction => todo!(),
            FormOperation::CancelAction => Method::DELETE,
        },
        |method| match method {
            http::Method::Get => Method::GET,
            http::Method::Put => Method::PUT,
            http::Method::Post => Method::POST,
            http::Method::Delete => Method::DELETE,
            http::Method::Patch => Method::PATCH,
        },
    )
}
