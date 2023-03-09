pub mod request;
pub mod try_access_request;

use std::{
    borrow::Cow,
    fmt::{self, Display},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{
    de::{self, Visitor},
    Deserialize, Deserializer, Serialize, Serializer,
};
use uuid::Uuid;

pub use self::try_access_request::TryAccessRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterInstance {
    pub purpose: RegisterInstancePurposee,
    pub message_id: Uuid,
    pub sub_topic_name: Arc<str>,
    pub sub_topic_uuid: Uuid,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RegisterInstancePurposee {
    #[default]
    Register,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterTryRequest {
    pub purpose: AfterTryRequestPurpose,
    pub message_id: Uuid,
    pub session_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AfterTryRequestPurpose {
    Start,
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub message_id: Uuid,
    #[serde(flatten)]
    pub inner: ResponseMessageInner,
}

// Needed to easily serialize and deserialize purpose tag.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "purpose", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseMessageInner {
    RegisterResponse {
        code: Code,
    },
    TryResponse {
        evaluation: Evaluation,
        session_id: Uuid,
    },
    StartResponse {
        evaluation: Evaluation,
    },
    EndResponse {
        evaluation: Evaluation,
    },
    ReevaluationResponse {
        evaluation: Evaluation,
        session_id: Uuid,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Evaluation {
    Permit,
    Deny,
}

impl Evaluation {
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Permit => "Permit",
            Self::Deny => "Deny",
        }
    }
}

impl Display for Evaluation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.to_str())
    }
}

impl Serialize for Evaluation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = match self {
            Evaluation::Permit => "permit",
            Evaluation::Deny => "deny",
        };

        s.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Evaluation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UconEvaluationVisitor;
        impl<'de> Visitor<'de> for UconEvaluationVisitor {
            type Value = Evaluation;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(if v.eq_ignore_ascii_case("permit") {
                    Evaluation::Permit
                } else {
                    Evaluation::Deny
                })
            }
        }

        deserializer.deserialize_str(UconEvaluationVisitor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    Ok,
    Ko,
}

impl Serialize for Code {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = match self {
            Code::Ok => "OK",
            Code::Ko => "KO",
        };

        s.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Code {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CodeVisitor;
        impl<'de> Visitor<'de> for CodeVisitor {
            type Value = Code;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a string")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(if v.eq_ignore_ascii_case("OK") {
                    Code::Ok
                } else {
                    Code::Ko
                })
            }
        }

        deserializer.deserialize_str(CodeVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload<'a, Command, Message> {
    #[serde(with = "serde_timestamp")]
    pub timestamp: SystemTime,
    pub command: GenericCommand<'a, Command, Message>,
}

impl<'a, Message> Payload<'a, RequestCommandType, Message> {
    pub fn pep_command(request_message: MessageContainer<'a, Message>) -> Self {
        let timestamp = SystemTime::now();
        Self {
            timestamp,
            command: RequestCommand {
                command_type: RequestCommandType::PepCommand,
                value: request_message,
            },
        }
    }
}

pub type Request<'a, Message> = Payload<'a, RequestCommandType, Message>;
pub type Response<'a> = Payload<'a, ResponseCommandType, ResponseMessage>;
pub type RequestCommand<'a, Message> = GenericCommand<'a, RequestCommandType, Message>;
pub type ResponseCommand<'a, Message> = GenericCommand<'a, ResponseCommandType, Message>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericCommand<'a, Command, Message> {
    pub command_type: Command,
    pub value: MessageContainer<'a, Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestCommandType {
    PepCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResponseCommandType {
    UcsCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageContainer<'a, T> {
    pub message: T,
    pub id: Cow<'a, str>,
    pub topic_name: Cow<'a, str>,
    pub topic_uuid: Cow<'a, str>,
}

mod serde_timestamp {
    use std::time::Duration;

    use super::{Deserialize, Deserializer, Serializer, SystemTime, UNIX_EPOCH};

    pub(super) fn serialize<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let duration = timestamp
            .duration_since(UNIX_EPOCH)
            .expect("timestamp is not allowed to be before unix epoch");

        // Sign is always positive, truncation is fine
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let timestamp = (duration.as_secs_f64() * 1000.) as u64;
        serializer.serialize_u64(timestamp)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_value = u64::deserialize(deserializer)?;
        let milliseconds = u32::try_from(raw_value % 1000).unwrap();
        let seconds = raw_value / 1000;
        Ok(UNIX_EPOCH + Duration::new(seconds, milliseconds * 1_000_000))
    }
}
