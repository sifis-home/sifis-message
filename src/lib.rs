mod deser;

use std::{
    borrow::Cow,
    fmt::{Debug, Write},
    path::Path,
};

use anyhow::Context;
use futures_util::Stream;
use rand::{thread_rng, Rng};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub async fn dht_cache_and_stream_from_config(
    config_path: &Path,
) -> anyhow::Result<(
    sifis_dht::cache::Cache,
    impl Stream<Item = sifis_dht::cache::Event>,
)> {
    let cache_config = toml_edit::de::from_slice(
        &tokio::fs::read(config_path)
            .await
            .context("unable to read cache TOML config file")?,
    )
    .context("unable to deserialize cache TOML config file")?;

    let (dht_cache, dht_events_stream) = sifis_dht::cache::Builder::from_config(cache_config)
        .make_channel()
        .await
        .context("unable to create dht cache")?;

    Ok((dht_cache, dht_events_stream))
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
