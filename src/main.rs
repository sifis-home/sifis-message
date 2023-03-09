mod response_handler;

use std::{
    future::ready,
    path::{Path, PathBuf},
    time::Duration,
};

use clap::Parser;
use futures_concurrency::future::Race;
use futures_util::StreamExt;
use log::{error, warn};
use rand::thread_rng;
use rsa::{pkcs8::EncodePrivateKey, RsaPrivateKey};
use serde::{Deserialize, Serialize};
use sifis_api::service::{Error as SifisApiError, SifisApi};
use sifis_dht::{
    domocache::{self, DomoCacheSender},
    domopersistentstorage::SqliteStorage,
};
use tarpc::{
    server::{BaseChannel, Channel},
    tokio_serde::formats::Bincode,
    tokio_util::codec::LengthDelimitedCodec,
};
use tokio::net::UnixListener;

#[derive(Debug, Clone)]
struct SifisToDht {
    cache_sender: DomoCacheSender,
    response_handler: response_handler::Sender,
}

const LAMP_TOPIC_NAME: &str = "Domo::Lamp";
const SINK_TOPIC_NAME: &str = "Domo::Sink";

#[derive(Debug, Deserialize)]
struct GetTopicNameEntry<T> {
    topic_name: String,
    topic_uuid: String,
    value: T,
}

#[derive(Debug, Deserialize)]
struct GetTopicNameResponse<T>(Vec<GetTopicNameEntry<T>>);

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PersistentMessage {
    Register,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "thingType")]
enum VolatileMessage {
    Lamp(LampMessage),
    Sink(SinkMessage),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LampMessage {
    op: LampMessageOp,
    uuid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum LampMessageOp {
    TurnOn,
    TurnOff,
    GetOnOff,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SinkMessage {
    uuid: String,
    op: SinkMessageOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum SinkMessageOp {}

impl SifisToDht {
    async fn find_by_topic(&self, topic_name: &str) -> Result<Vec<String>, SifisApiError> {
        let raw_topic_names: GetTopicNameResponse<serde_json::Value> = serde_json::from_value(
            self.cache_sender
                .get_topic_name(topic_name)
                .await
                .map_err(|_| SifisApiError::Forbidden("cannot read topic from cache".to_owned()))?,
        )
        .map_err(|_| SifisApiError::Forbidden("cannot deserialize cache response".to_owned()))?;

        let out = raw_topic_names
            .0
            .into_iter()
            .filter_map(|topic| {
                serde_json::from_value(topic.value)
                    .ok()
                    .filter(|message| matches!(message, PersistentMessage::Register))
                    .map(|_| topic.topic_uuid)
            })
            .fold(Vec::new(), |mut lamps, uuid| {
                if let Err(index) = lamps.binary_search(&uuid) {
                    lamps.insert(index, uuid);
                }
                lamps
            });

        Ok(out)
    }
}

#[tarpc::server]
impl SifisApi for SifisToDht {
    #[inline]
    async fn find_lamps(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        self.find_by_topic(LAMP_TOPIC_NAME).await
    }

    async fn turn_lamp_on(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn turn_lamp_off(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn get_lamp_on_off(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        // TODO
        self.response_handler.lamp_on_off(id).await
    }

    async fn set_lamp_brightness(
        self,
        _: tarpc::context::Context,
        id: String,
        brightness: u8,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    async fn get_lamp_brightness(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    #[inline]
    async fn find_sinks(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        self.find_by_topic(SINK_TOPIC_NAME).await
    }

    async fn set_sink_flow(
        self,
        _: tarpc::context::Context,
        id: String,
        flow: u8,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    async fn get_sink_flow(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    async fn set_sink_temp(
        self,
        _: tarpc::context::Context,
        id: String,
        temp: u8,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    async fn get_sink_temp(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }

    async fn close_sink_drain(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn open_sink_drain(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn get_sink_level(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        todo!()
    }
}

#[derive(Parser, Debug)]
struct Cli {
    /// Path to a sqlite file
    sqlite_file: PathBuf,

    /// Path to a private key file
    private_key_file: PathBuf,

    /// Use a persistent cache
    is_persistent_cache: bool,

    /// 32 bytes long shared key in hex format
    shared_key: String,

    /// use only loopback iface for libp2p
    loopback_only: bool,

    /// Unix socket to bind to
    #[clap(short, long, default_value = "/var/run/sifis.sock")]
    socket: PathBuf,

    /// size of message buffer
    #[clap(short, long)]
    message_buffer_size: Option<usize>,
}

fn get_or_write_key_pair(
    private_key_file: &Path,
) -> Result<libp2p::identity::Keypair, Box<dyn std::error::Error>> {
    if private_key_file.try_exists()? {
        let raw_pem = std::fs::read(private_key_file)?;
        let (_label, mut der) = pem_rfc7468::decode_vec(&raw_pem)
            .map_err(|_| "cannot decode private key file as PEM")?;
        let keypair = libp2p::identity::Keypair::rsa_from_pkcs8(&mut der)?;
        Ok(keypair)
    } else {
        let mut rng = thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
        let mut der = private_key.to_pkcs8_der()?.to_bytes();
        let keypair = libp2p::identity::Keypair::rsa_from_pkcs8(&mut der)?;
        Ok(keypair)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();
    let codec_builder = LengthDelimitedCodec::builder();
    let listener = UnixListener::bind(cli.socket).expect("unable to open Unix socket {bind_addr}");

    let storage = SqliteStorage::new(cli.sqlite_file, cli.is_persistent_cache);
    let key_pair = get_or_write_key_pair(&cli.private_key_file)?;
    let (cache_sender, cache_receiver) = domocache::channel(
        cli.is_persistent_cache,
        storage,
        cli.shared_key,
        key_pair,
        false,
        cli.message_buffer_size,
    )
    .await;
    let sifis_to_dht = SifisToDht { cache_sender };
    let receive_future = async move {
        loop {
            let conn = match listener.accept().await {
                Ok((conn, _addr)) => conn,
                Err(err) => {
                    warn!("unable to accept connection from Unix socket: {err}");
                    // Just a little wait time to avoid a tight loop
                    tokio::time::sleep(Duration::from_millis(15)).await;
                    continue;
                }
            };
            let framed = codec_builder.new_framed(conn);
            let transport = tarpc::serde_transport::new(framed, Bincode::default());

            let fut = BaseChannel::with_defaults(transport).execute(sifis_to_dht.clone().serve());
            tokio::spawn(fut);
        }
    };

    (
        cache_receiver.into_stream().for_each(|result| {
            if let Err(err) = result {
                error!("Error from domo cache: {err}");
            }

            ready(())
        }),
        receive_future,
    )
        .race()
        .await;

    Ok(())
}
