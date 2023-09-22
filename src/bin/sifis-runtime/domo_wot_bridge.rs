use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Topic<'a, T> {
    pub topic_name: &'a str,
    pub topic_uuid: &'a str,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Lamp {
    pub status: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct VolatileMessage<'a> {
    #[serde(serialize_with = "serialize_timestamp")]
    pub timestamp: SystemTime,
    pub command: Command<'a>,
}

impl<'a> VolatileMessage<'a> {
    pub fn new(command: Command<'a>) -> Self {
        let timestamp = SystemTime::now();
        Self { timestamp, command }
    }
}

fn serialize_timestamp<S>(timestamp: &SystemTime, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let duration = timestamp
        .duration_since(UNIX_EPOCH)
        .expect("timestamps before unix epochs are invalid");
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let value = (duration.as_secs_f64() * 1000.) as u64;
    value.serialize(serializer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(tag = "command_type", content = "value", rename_all = "snake_case")]
pub enum Command<'a> {
    TurnCommand(CommandInner<'a, TurnCommand>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct CommandInner<'a, T> {
    pub topic_name: &'a str,
    pub topic_uuid: &'a str,
    #[serde(flatten)]
    pub inner: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct TurnCommand {
    pub desired_state: bool,
}
