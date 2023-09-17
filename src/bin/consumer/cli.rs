use std::{
    error::Error as StdError,
    fmt::{self, Display},
    path::PathBuf,
    str::FromStr,
};

use clap::Parser;
use uuid::Uuid;

#[derive(Debug, Parser)]
pub struct Cli {
    /// Path to config for libp2p-rust-dht cache in TOML format
    pub cache_config_file: PathBuf,

    /// One or multiple things executables.
    ///
    /// The format for each item is `/path/of/bin:port:type`, where:
    /// - `port` is the port the thing will bind to.
    /// - `type` can be "lamp" or "sink".
    ///
    /// Keep in mind that the CLI parameter `--listen-port` will be used when running the binary in
    /// order to specify the port.
    pub things: Vec<Thing>,
}

#[derive(Debug, Clone)]
pub struct Thing {
    pub path: PathBuf,
    pub port: u16,
    pub ty: ThingType,
    pub uuid: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThingType {
    Lamp,
    Sink,
}

#[derive(Debug)]
pub enum InvalidThing {
    MissingColon(String),
    Port(String),
    Type(InvalidThingType),
    Uuid(uuid::Error),
}

impl FromStr for Thing {
    type Err = InvalidThing;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut split = s.splitn(3, ':');

        macro_rules! get {
            () => {
                split
                    .next()
                    .ok_or_else(|| InvalidThing::MissingColon(s.to_owned()))?
            };
        }

        let raw_path = get!();
        let raw_port = get!();
        let raw_type = get!();
        let raw_uuid = get!();

        let path = PathBuf::from(raw_path);
        let port = raw_port
            .parse()
            .map_err(|_| InvalidThing::Port(s.to_owned()))?;
        let ty = raw_type.parse().map_err(InvalidThing::Type)?;
        let uuid = Uuid::parse_str(raw_uuid).map_err(InvalidThing::Uuid)?;

        Ok(Self {
            path,
            port,
            ty,
            uuid,
        })
    }
}

impl Display for InvalidThing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvalidThing::MissingColon(s) => write!(
                f,
                r#"missing colon to separate binary and port in CLI thing "{s}""#,
            ),
            InvalidThing::Port(s) => write!(f, r#"port is not valid in CLI thing "{s}""#),
            InvalidThing::Type(_) => f.write_str("invalid thing type given"),
            InvalidThing::Uuid(_) => f.write_str("invalid uuid format"),
        }
    }
}

impl StdError for InvalidThing {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            InvalidThing::Type(source) => Some(source),
            InvalidThing::Uuid(err) => Some(err),
            InvalidThing::MissingColon(_) | InvalidThing::Port(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidThingType(String);

impl FromStr for ThingType {
    type Err = InvalidThingType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lamp" => Ok(Self::Lamp),
            "sink" => Ok(Self::Sink),
            _ => Err(InvalidThingType(s.to_owned())),
        }
    }
}

impl Display for InvalidThingType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#""{}" is not a valid thing type"#, self.0)
    }
}

impl StdError for InvalidThingType {}
