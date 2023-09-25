use std::{borrow::Cow, sync::Arc, time::SystemTime};

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sifis_dht::domocache::{DomoCache, DomoEvent};
use sifis_message::manage_domo_cache;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ucs::{self, Evaluation, Payload, ResponseMessageInner};

#[derive(Debug)]
pub struct Mock {
    cache_config: sifis_config::Cache,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
}

impl Mock {
    pub async fn start(&mut self) -> Result<(), Error> {
        let domo_cache = DomoCache::new(cache_config)
            .await
            .map_err(Error::DomoCacheCreation)?;

        let (cache_sender, cache_receiver) = mpsc::channel(32);
        let (response_sender, response_receiver) = mpsc::unbounded_channel();

        manage_domo_cache(domo_cache, cache_receiver, {
            move |message, domo_cache| handle_message(message, domo_cache).await
        })
        .map_err(Error::ManageDomoCache)
        .try_for_each(|event| handle_domo_event(event, response_sender))
        .await?;

        todo!()
    }
}

#[derive(Debug)]
pub enum Error {
    DomoCacheCreation(Box<dyn std::error::Error>),
    ManageDomoCache(Box<dyn std::error::Error>),
    ResponseChannelClosed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DummyMessage;

async fn handle_message(_dummy: DummyMessage, domo_cache: &mut DomoCache) -> Result<(), Error> {
    todo!()
}

async fn handle_domo_event(
    event: DomoEvent,
    response_sender: mpsc::UnboundedSender<ucs::Response<'static>>,
    ucs_topic_name: Arc<str>,
    ucs_topic_uuid: Arc<str>,
) -> Result<(), Error> {
    let DomoEvent::VolatileData(volatile) = event else {
        return Ok(());
    };

    let Ok(payload) = ucs::Payload::deserialize(&volatile) else {
        return Ok(());
    };

    let Payload {
        timestamp: _,
        command:
            ucs::RequestCommand {
                command_type: ucs::RequestCommandType::PepCommand,
                value:
                    ucs::MessageContainer {
                        message,
                        id,
                        topic_name,
                        topic_uuid,
                    },
            },
    } = payload;

    let message_inner = match message {
        UcsMessage::RegisterInstance(ucs::RegisterInstance {
            purpose: _,
            message_id: _,
            sub_topic_name,
            sub_topic_uuid,
        }) => ResponseMessageInner::RegisterResponse {
            code: ucs::Code::Ok,
        },

        UcsMessage::AfterTryRequest(after_try_request) => ResponseMessageInner::TryResponse {
            evaluation: Evaluation::Permit,
            session_id: Uuid::new_v4(),
        },
    };

    let response = ucs::Response {
        timestamp: SystemTime::now(),
        command: ucs::GenericCommand {
            command_type: ucs::ResponseCommandType::UcsCommand,
            value: ucs::MessageContainer {
                message: ucs::ResponseMessage {
                    message_id: Uuid::new_v4(),
                    inner: message_inner,
                },
                id: Cow::Borrowed("ucs-mock"),
                topic_name: Cow::Owned(ucs_topic_name.as_ref().to_owned()),
                topic_uuid: Cow::Owned(ucs_topic_uuid.as_ref().to_owned()),
            },
        },
    };

    response_sender
        .send(response)
        .map_err(|_| Error::ResponseChannelClosed)?;

    todo!()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum UcsMessage {
    RegisterInstance(ucs::RegisterInstance),
    AfterTryRequest(ucs::AfterTryRequest),
}
