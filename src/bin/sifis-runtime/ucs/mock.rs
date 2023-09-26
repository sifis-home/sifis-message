use std::{
    borrow::Cow,
    convert::Infallible,
    fmt::{self, Display},
    future,
    time::SystemTime,
};

use futures_concurrency::future::Race;
use futures_util::{FutureExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use sifis_dht::domocache::{DomoCache, DomoEvent};
use sifis_message::manage_domo_cache;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::ucs::{self, Evaluation, Payload, ResponseMessageInner};

#[derive(Debug)]
pub struct Mock {
    pub cache_config: sifis_config::Cache,
    pub ucs_topic_name: String,
    pub ucs_topic_uuid: String,
}

impl Mock {
    pub async fn start(self) -> Result<Infallible, Error> {
        let Self {
            cache_config,
            ucs_topic_name,
            ucs_topic_uuid,
        } = self;

        let domo_cache = DomoCache::new(cache_config)
            .await
            .map_err(Error::DomoCacheCreation)?;

        let (cache_sender, cache_receiver) = mpsc::channel(32);
        let (response_sender, response_receiver) = mpsc::unbounded_channel();

        (
            Box::pin(
                manage_domo_cache(domo_cache, cache_receiver, {
                    move |message, domo_cache| handle_message(message, domo_cache).boxed()
                })
                .map_err(Error::ManageDomoCache)
                .try_for_each(move |event| {
                    future::ready(handle_domo_event(
                        event,
                        &response_sender,
                        &ucs_topic_name,
                        &ucs_topic_uuid,
                    ))
                }),
            ),
            handle_responses(response_receiver, cache_sender),
        )
            .race()
            .await?;

        unreachable!("UCS mock is not expected to terminate");
    }
}

#[derive(Debug)]
pub enum Error {
    DomoCacheCreation(Box<dyn std::error::Error + 'static>),
    ManageDomoCache(Box<dyn std::error::Error + 'static>),
    ResponseChannelClosed,
    CacheChannelClosed,
    ResponseToJson(serde_json::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DomoCacheCreation(_) => f.write_str("unable to create domo cache instance"),
            Error::ManageDomoCache(_) => f.write_str("error while handling domo cache"),
            Error::ResponseChannelClosed => {
                f.write_str("response channel to interact with domo cache is unexpectedly closed")
            }
            Error::CacheChannelClosed => {
                f.write_str("domo cache channel to interact with domo cache is unexpectedly closed")
            }
            Error::ResponseToJson(_) => {
                f.write_str("unable to convert UCS response message to JSON")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::DomoCacheCreation(err) | Error::ManageDomoCache(err) => Some(&**err),
            Error::ResponseToJson(err) => Some(err),
            Error::ResponseChannelClosed | Error::CacheChannelClosed => None,
        }
    }
}

async fn handle_message(
    message: ucs::Response<'static>,
    domo_cache: &mut DomoCache,
) -> Result<(), Error> {
    let message = serde_json::to_value(message).map_err(Error::ResponseToJson)?;
    domo_cache.pub_value(message).await;
    Ok(())
}

fn handle_domo_event(
    event: DomoEvent,
    response_sender: &mpsc::UnboundedSender<ucs::Response<'static>>,
    ucs_topic_name: &str,
    ucs_topic_uuid: &str,
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
                        id: _,
                        topic_name,
                        topic_uuid,
                    },
            },
    } = payload;

    if topic_name != ucs_topic_name || topic_uuid != ucs_topic_uuid {
        return Ok(());
    }

    let message_inner = match message {
        UcsMessage::RegisterInstance(_) => ResponseMessageInner::RegisterResponse {
            code: ucs::Code::Ok,
        },

        UcsMessage::TryAccessRequest(_) => ResponseMessageInner::TryResponse {
            evaluation: Evaluation::Permit,
            session_id: Uuid::new_v4(),
        },

        UcsMessage::AfterTryRequest(ucs::AfterTryRequest {
            purpose: ucs::AfterTryRequestPurpose::Start,
            ..
        }) => ResponseMessageInner::StartResponse {
            evaluation: Evaluation::Permit,
        },

        UcsMessage::AfterTryRequest(ucs::AfterTryRequest {
            purpose: ucs::AfterTryRequestPurpose::End,
            ..
        }) => ResponseMessageInner::EndResponse {
            evaluation: Evaluation::Permit,
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
                topic_name: Cow::Owned(ucs_topic_name.to_owned()),
                topic_uuid: Cow::Owned(ucs_topic_uuid.to_owned()),
            },
        },
    };

    response_sender
        .send(response)
        .map_err(|_| Error::ResponseChannelClosed)?;

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum UcsMessage {
    RegisterInstance(ucs::RegisterInstance),
    TryAccessRequest(DummyTryAccessRequest),
    AfterTryRequest(ucs::AfterTryRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DummyTryAccessRequest {
    pub message_id: Uuid,
    pub request: Cow<'static, str>,
}

pub async fn handle_responses(
    mut response_receiver: mpsc::UnboundedReceiver<ucs::Response<'static>>,
    cache_sender: mpsc::Sender<ucs::Response<'static>>,
) -> Result<(), Error> {
    loop {
        let response = response_receiver
            .try_recv()
            .map_err(|_| Error::ResponseChannelClosed)?;

        cache_sender
            .send(response)
            .await
            .map_err(|_| Error::CacheChannelClosed)?;
    }
}
