mod deser;

use std::{
    borrow::Cow,
    fmt::{Debug, Write},
    future::Future,
    path::Path,
    pin::Pin,
};

use anyhow::{anyhow, Context};
use futures_util::{stream, Stream};
use log::debug;
use rand::{thread_rng, Rng};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sifis_dht::domocache::{DomoCache, DomoEvent};
use tokio::{select, sync::mpsc};
use uuid::Uuid;

pub async fn domo_cache_from_config(config_path: &Path) -> anyhow::Result<DomoCache> {
    let cache_config = toml_edit::de::from_slice(
        &tokio::fs::read(config_path)
            .await
            .context("unable to read cache TOML config file")?,
    )
    .context("unable to deserialize cache TOML config file")?;

    let domo_cache = DomoCache::new(cache_config)
        .await
        .map_err(|err| anyhow!("unable to create domo cache: {err:?}"))?;

    Ok(domo_cache)
}

pub async fn dump_sample_domo_cache(config_path: &Path) -> anyhow::Result<()> {
    let mut rng = thread_rng();
    let mut shared_key = String::with_capacity(64);
    for _ in 0..32 {
        write!(shared_key, "{:02x}", rng.gen::<u8>()).unwrap();
    }

    let config = sifis_config::Cache {
        url: "sqlite::memory:".to_string(),
        table: "dht".to_string(),
        persistent: true,
        private_key: None,
        shared_key,
        loopback: true,
    };

    let config_string = toml_edit::ser::to_string_pretty(&config)
        .context("unable to convert sifis cache config to TOML")?;
    tokio::fs::write(config_path, &config_string)
        .await
        .context("unable to write sifis cache config to TOML file")?;
    Ok(())
}

pub fn manage_domo_cache<T, E, F>(
    domo_cache: DomoCache,
    message_receiver: mpsc::Receiver<T>,
    message_handler: F,
) -> impl Stream<Item = Result<DomoEvent, E>>
where
    T: 'static + Debug,
    sifis_dht::Error: Into<E>,
    F: for<'a> Fn(T, &'a mut DomoCache) -> Pin<Box<dyn Future<Output = Result<(), E>> + 'a>>
        + Clone,
{
    stream::try_unfold(
        (domo_cache, message_receiver),
        move |(domo_cache, receiver)| {
            manage_domo_cache_inner(domo_cache, receiver, message_handler.clone())
        },
    )
}

async fn manage_domo_cache_inner<T, E, F>(
    mut domo_cache: DomoCache,
    mut message_receiver: mpsc::Receiver<T>,
    message_handler: F,
) -> Result<Option<(DomoEvent, (DomoCache, mpsc::Receiver<T>))>, E>
where
    T: 'static + Debug,
    sifis_dht::Error: Into<E>,
    F: for<'a> Fn(T, &'a mut DomoCache) -> Pin<Box<dyn Future<Output = Result<(), E>> + 'a>>,
{
    loop {
        select! {
            result = domo_cache.cache_event_loop() => {
                debug!("Received event from domo cache: {result:#?}");
                return result.map(|domo_event| {
                    Some((domo_event, (domo_cache, message_receiver)))
                }).map_err(Into::into);
            }

            // This is cancel safe, therefore it is fine
            message = message_receiver.recv() => {
                debug!("Received message from domo receiver: {message:#?}");
                match message {
                    Some(message) => {
                        message_handler(message, &mut domo_cache).await?;
                    },
                    None => break,
                }
            }
        }
    }

    domo_cache
        .cache_event_loop()
        .await
        .map(|domo_event| Some((domo_event, (domo_cache, message_receiver))))
        .map_err(Into::into)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestMessage {
    pub message_type: RequestMessageType,

    pub request_id: Uuid,

    pub thing_id: Uuid,

    #[serde(with = "crate::deser::method")]
    pub method: reqwest::Method,

    pub path: Cow<'static, str>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<Authorization>,

    #[serde(
        skip_serializing_if = "Vec::is_empty",
        with = "crate::deser::headers",
        default
    )]
    pub headers: Vec<(HeaderName, HeaderValue)>,

    #[serde(skip_serializing_if = "MessageBody::is_empty", default)]
    pub body: MessageBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestMessageType {
    RequestMessage,
}

impl RequestMessage {
    #[inline]
    pub fn new(thing_id: Uuid, path: impl Into<Cow<'static, str>>) -> Self {
        Self {
            message_type: RequestMessageType::RequestMessage,
            request_id: Uuid::new_v4(),
            thing_id,
            method: reqwest::Method::GET,
            path: path.into(),
            authorization: None,
            headers: Vec::new(),
            body: MessageBody::default(),
        }
    }

    pub fn set_method(&mut self, method: reqwest::Method) -> &mut Self {
        self.method = method;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Authorization {
    Basic { username: String, password: String },
    Bearer(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageBody {
    String(String),
    Bytes(Vec<u8>),
}

impl MessageBody {
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            MessageBody::String(s) => s.is_empty(),
            MessageBody::Bytes(b) => b.is_empty(),
        }
    }
}

impl Default for MessageBody {
    #[inline]
    fn default() -> Self {
        MessageBody::Bytes(Vec::new())
    }
}

impl From<MessageBody> for reqwest::Body {
    fn from(value: MessageBody) -> Self {
        match value {
            MessageBody::String(s) => s.into(),
            MessageBody::Bytes(b) => b.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InternalResponseMessage {
    pub message_type: ResponseMessageType,
    pub request_id: Uuid,
    pub body: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseMessageType {
    ResponseMessage,
}

pub const LAMP_TOPIC_NAME: &str = "sifis_lamp";
pub const SINK_TOPIC_NAME: &str = "sifis_sink";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GetTopicNameEntry<T> {
    pub topic_name: String,
    pub topic_uuid: String,
    pub value: T,
}
