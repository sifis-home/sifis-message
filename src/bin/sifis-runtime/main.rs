#![warn(clippy::pedantic)]

mod continuation;
mod registration;
mod registration_handler;
mod ucs;

use std::{
    any,
    collections::HashMap,
    ffi::c_int,
    fmt::{self, Display},
    ops::Not,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    sync::{
        atomic::{self, AtomicBool},
        Arc,
    },
    time::Duration,
};

use anyhow::{anyhow, bail, Context};
use clap::Parser;
use continuation::Continuation;
use futures_concurrency::future::Race;
use futures_util::{FutureExt, TryStreamExt};
use log::{debug, error, info, trace, warn};
use registration::{Registration, ResponseError, ResponseSender};
use registration_handler::handle_registration_message;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sifis_api::{
    service::{Error as SifisApiError, SifisApi},
    DoorLockStatus, Hazard,
};
use sifis_dht::domocache::DomoEvent;
use sifis_message::{
    domo_cache_from_config, dump_sample_domo_cache, manage_domo_cache, GetTopicNameEntry,
    InternalResponseMessage, LAMP_TOPIC_NAME, SINK_TOPIC_NAME,
};
use tarpc::{
    server::{BaseChannel, Channel},
    tokio_serde::formats::Bincode,
    tokio_util::codec::LengthDelimitedCodec,
};
use tokio::{
    net::UnixListener,
    sync::{mpsc, oneshot},
    time::{self, sleep, Instant},
};
use uuid::Uuid;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct SifisToDht {
    cache_sender: mpsc::Sender<Registration>,
    peer_id: PeerId,
}

#[derive(Debug, Deserialize)]
struct GetTopicNameResponse<T>(Vec<GetTopicNameEntry<T>>);

impl SifisToDht {
    async fn register<T>(
        &self,
        (registration, receiver): (
            Registration,
            registration::Receiver<Result<T, registration::ResponseError>>,
        ),
    ) -> Result<T, SifisApiError> {
        self.cache_sender.send(registration).await.map_err(|_| {
            SifisApiError::NotFound("unable to send data through domo cache channel".to_owned())
        })?;

        match receiver.await {
            Ok(Ok(data)) => Ok(data),
            Ok(Err(registration::ResponseError)) => Err(SifisApiError::Forbidden {
                risk: Hazard::Fire,
                comment: "dummy hazard".to_owned(),
            }),
            Err(registration::ReceiverError::Recv) => Err(SifisApiError::NotFound(
                "domo response channel closed".to_owned(),
            )),
            Err(registration::ReceiverError::Timeout) => Err(SifisApiError::NotFound(
                "no reply from domo cache".to_string(),
            )),
        }
    }
}

#[tarpc::server]
impl SifisApi for SifisToDht {
    #[inline]
    async fn find_lamps(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        trace!("Request to find lamps");
        find_by_topic(&self.cache_sender, LAMP_TOPIC_NAME).await
    }

    async fn turn_lamp_on(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        trace!("Request to turn lamp on for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::lamp_set_on(id, true, self.peer_id))
            .await?;
        Ok(true)
    }

    async fn turn_lamp_off(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        trace!("Request to turn lamp off for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::lamp_set_on(id, false, self.peer_id))
            .await?;
        Ok(false)
    }

    async fn get_lamp_on_off(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        trace!("Request the lamp on/off status for {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::lamp_on_off(id, self.peer_id))
            .await
    }

    async fn set_lamp_brightness(
        self,
        _: tarpc::context::Context,
        id: String,
        brightness: u8,
    ) -> Result<u8, SifisApiError> {
        trace!("Request to set the lamp brightness for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::lamp_set_brightness(
            id,
            brightness,
            self.peer_id,
        ))
        .await?;
        Ok(brightness)
    }

    async fn get_lamp_brightness(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        trace!("Request the lamp brightness for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::lamp_brightness(id, self.peer_id))
            .await
    }

    #[inline]
    async fn find_sinks(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        trace!("Request to find sinks");
        find_by_topic(&self.cache_sender, SINK_TOPIC_NAME).await
    }

    async fn set_sink_flow(
        self,
        _: tarpc::context::Context,
        id: String,
        flow: u8,
    ) -> Result<u8, SifisApiError> {
        trace!("Request to set sink flow for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_set_flow(id, flow, self.peer_id))
            .await?;
        Ok(flow)
    }

    async fn get_sink_flow(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        trace!("Request the sink flow for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_flow(id, self.peer_id))
            .await
    }

    async fn set_sink_temp(
        self,
        _: tarpc::context::Context,
        id: String,
        temp: u8,
    ) -> Result<u8, SifisApiError> {
        trace!("Request to set sink temperature for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_set_temp(id, temp, self.peer_id))
            .await?;
        Ok(temp)
    }

    async fn get_sink_temp(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        trace!("Request the sink temperature for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_temp(id, self.peer_id))
            .await
    }

    async fn close_sink_drain(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        trace!("Request to close the sink drain for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_set_drain(id, false, self.peer_id))
            .await?;
        Ok(false)
    }

    async fn open_sink_drain(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<bool, SifisApiError> {
        trace!("Request to open the sink drain for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_set_drain(id, true, self.peer_id))
            .await?;
        Ok(true)
    }

    async fn get_sink_level(
        self,
        _: tarpc::context::Context,
        id: String,
    ) -> Result<u8, SifisApiError> {
        trace!("Request the sink level for id {id}");
        let id = from_str_uuid(&id)?;
        self.register(Registration::sink_level(id, self.peer_id))
            .await
    }

    async fn find_doors(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        todo!()
    }

    async fn get_door_lock_status(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<DoorLockStatus, SifisApiError> {
        todo!()
    }

    async fn get_door_open(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn lock_door(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn unlock_door(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn find_fridges(self, _: tarpc::context::Context) -> Result<Vec<String>, SifisApiError> {
        todo!()
    }

    async fn get_fridge_open(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<bool, SifisApiError> {
        todo!()
    }

    async fn get_fridge_target_temperature(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<i8, SifisApiError> {
        todo!()
    }

    async fn get_fridge_temperature(
        self,
        _: tarpc::context::Context,
        _id: String,
    ) -> Result<i8, SifisApiError> {
        todo!()
    }

    async fn set_fridge_target_temperature(
        self,
        _: tarpc::context::Context,
        _id: String,
        _target_temperature: i8,
    ) -> Result<i8, SifisApiError> {
        todo!()
    }
}

fn from_str_uuid(id: &str) -> Result<Uuid, SifisApiError> {
    id.parse()
        .map_err(|_| SifisApiError::NotFound("invalid id".to_owned()))
}

#[derive(Parser, Debug)]
struct Cli {
    /// Path to config for libp2p-rust-dht cache in TOML format
    cache_config_file: PathBuf,

    /// Dump a sample config and exit.
    #[clap(short, long)]
    dump: bool,

    /// Unix socket to bind to
    #[clap(
        short,
        long,
        default_value = "/var/run/sifis.sock",
        env = "SIFIS_SERVER"
    )]
    socket: PathBuf,

    /// The name used to register to UCS.
    #[clap(long)]
    client_id: Arc<str>,

    /// The topic name we expect to receive messages from UCS.
    #[clap(long)]
    topic_name: Arc<str>,

    /// The topic uuid we expect to receive messages from UCS.
    #[clap(long)]
    topic_uuid: Option<Uuid>,

    /// The topic name used by UCS.
    #[clap(long)]
    ucs_topic_name: Arc<str>,

    /// The topic uuid used by UCS.
    #[clap(long)]
    ucs_topic_uuid: Arc<str>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    if cli.dump {
        dump_sample_domo_cache(&cli.cache_config_file).await?;
        println!(
            "Config successfully dumped to {}",
            cli.cache_config_file.display()
        );
        return Ok(());
    }

    if Path::new(&cli.socket).exists() {
        std::fs::remove_file(&cli.socket)
            .unwrap_or_else(|err| panic!("unable to remove old unix socket file: {err}"));
    }

    let domo_cache = domo_cache_from_config(&cli.cache_config_file).await?;
    let (cache_sender, cache_receiver) = mpsc::channel(32);
    let (response_sender, response_receiver) = mpsc::channel(32);
    let (continuation_sender, continuation_receiver) = mpsc::unbounded_channel();

    let tarpc_server = {
        let cache_sender = cache_sender.clone();
        create_tarpc_server(&cli.socket, move |peer_id| {
            let cache_sender = cache_sender.clone();
            SifisToDht {
                cache_sender,
                peer_id,
            }
            .serve()
        })
    };

    let Cli {
        client_id,
        topic_name,
        topic_uuid,
        ucs_topic_name,
        ucs_topic_uuid,
        ..
    } = cli;
    let topic_uuid = topic_uuid.unwrap_or_else(Uuid::new_v4);
    let topic_uuid_str = topic_uuid.to_string();

    let registered_to_ucs = AtomicBool::new(false);
    (
        Box::pin(manage_domo_cache(domo_cache, cache_receiver, {
            let response_sender = response_sender.clone();
            let continuation_sender = continuation_sender.clone();
            let ucs_topic_name = Arc::clone(&ucs_topic_name);
            move |message, domo_cache| {
                handle_registration_message(
                    message,
                    domo_cache,
                    Arc::clone(&client_id),
                    Arc::clone(&ucs_topic_name),
                    Arc::clone(&ucs_topic_uuid),
                    response_sender.clone(),
                    continuation_sender.clone(),
                )
                .boxed()
            }
        }))
        .map_err(|err| anyhow!("error while managing domo cache: {err}"))
        .try_for_each(|event| {
            handle_domo_event(event, &topic_name, &topic_uuid_str, &response_sender)
        }),
        handle_responses(
            response_receiver,
            &registered_to_ucs,
            continuation_sender.clone(),
        ),
        tarpc_server.map(Ok),
        continuation::handle_receiver(continuation_receiver, &response_sender, &cache_sender),
        registration_handler(&cache_sender, &registered_to_ucs, &topic_name, topic_uuid).map(Ok),
    )
        .race()
        .await?;

    Ok(())
}

async fn handle_domo_event(
    event: DomoEvent,
    topic_name: &str,
    topic_uuid_str: &str,
    response_sender: &mpsc::Sender<QueuedResponseMessage>,
) -> anyhow::Result<()> {
    let DomoEvent::VolatileData(volatile_data) = event else {
        return Ok(());
    };

    let Ok(response_message) = serde_json::from_value(volatile_data) else {
        return Ok(());
    };

    if matches!(
        &response_message,
        ResponseMessage::Ucs(ucs::Response {
            command:
                ucs::ResponseCommand {
                    value:
                        ucs::MessageContainer {
                            topic_name: response_topic_name,
                            topic_uuid: response_topic_uuid,
                            ..
                        },
                    ..
                },
            ..
        })
        if response_topic_name != topic_name || response_topic_uuid != topic_uuid_str
    ) {
        return Ok(());
    }

    trace!("Got response message from volatile data: {response_message:#?}");
    response_sender
        .send(QueuedResponseMessage::Respond(response_message))
        .await
        .context("response channel should not be closed")?;
    Ok(())
}

async fn create_tarpc_server<F, S, Req>(socket_path: &Path, mut serve_fn: F)
where
    F: FnMut(PeerId) -> S,
    S: tarpc::server::Serve<Req> + Send + Clone + 'static,
    Req: DeserializeOwned + Send + 'static,
    S::Resp: Serialize + Send,
    S::Fut: Send,
{
    let codec_builder = LengthDelimitedCodec::builder();
    let listener = UnixListener::bind(socket_path).expect("unable to open Unix socket");

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
        let peer = transport.get_ref();
        let peer_id = PeerId(sifis_api::runtime::peer_pid(peer.as_raw_fd()));

        let fut = BaseChannel::with_defaults(transport).execute(serve_fn(peer_id));
        tokio::spawn(fut);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
struct PeerId(c_int);

impl Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

async fn find_by_topic(
    cache_sender: &mpsc::Sender<Registration>,
    topic_name: impl Into<String>,
) -> Result<Vec<String>, SifisApiError> {
    let (registration, response_receiver) = Registration::get_topic_name(topic_name);
    cache_sender
        .send(registration)
        .await
        .expect("unable to write into cache wrapper channel");

    let raw_topic_names = response_receiver
        .await
        .expect("unable to get message from responder")
        .map_err(SifisApiError::NotFound)?;

    debug!("Received response for get_topic_name: {raw_topic_names:#?}");

    serde_json::from_value::<GetTopicNameResponse<()>>(raw_topic_names)
        .map_err(|err| SifisApiError::NotFound(err.to_string()))
        .map(|entries| {
            entries
                .0
                .into_iter()
                .map(|entry| entry.topic_uuid)
                .collect()
        })
}

async fn handle_responses(
    mut response_receiver: mpsc::Receiver<QueuedResponseMessage>,
    registered_to_ucs: &AtomicBool,
    continuation_sender: mpsc::UnboundedSender<Continuation>,
) -> anyhow::Result<()> {
    const ENTRY_DURATION: Duration = Duration::from_secs(10);
    const REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

    let mut queue = HashMap::<Uuid, ResponseQueueEntry>::new();
    let mut last_cleanup = Instant::now();
    let mut responses_states = ResponseStates::default();

    loop {
        while queue.is_empty().not() {
            match time::timeout(ENTRY_DURATION, response_receiver.recv()).await {
                Ok(Some(message)) => handle_response(
                    message,
                    &mut queue,
                    &mut last_cleanup,
                    &mut responses_states,
                    registered_to_ucs,
                    &continuation_sender,
                ),
                Err(_) => {
                    let now = Instant::now();

                    if matches!(
                        responses_states.registration_data,
                        Some(RegistrationData { instant, .. })
                        if instant + REGISTRATION_TIMEOUT >= now
                    ) {
                        trace!("Registration to UCS timed out");

                        responses_states.registration_data = None;
                        registered_to_ucs.store(false, atomic::Ordering::Release);
                    }

                    queue.retain(|_, entry| entry.expiration > now);
                    last_cleanup = now;
                }
                Ok(None) => bail!("responses channel is unexpectedly closed"),
            }
        }

        debug!("Queue is empty, waiting a message");
        last_cleanup = Instant::now();
        let message = response_receiver
            .recv()
            .await
            .ok_or_else(|| anyhow!("responses channel is unexpectedly closed"))?;

        handle_response(
            message,
            &mut queue,
            &mut last_cleanup,
            &mut responses_states,
            registered_to_ucs,
            &continuation_sender,
        );
    }
}

#[derive(Debug)]
struct ResponseQueueEntry {
    responder: Responder,
    expiration: Instant,
}

fn handle_response(
    message: QueuedResponseMessage,
    queue: &mut HashMap<Uuid, ResponseQueueEntry>,
    last_cleanup: &mut Instant,
    responses_states: &mut ResponseStates,
    registered_to_ucs: &AtomicBool,
    continuation_sender: &mpsc::UnboundedSender<Continuation>,
) {
    match message {
        QueuedResponseMessage::Register {
            uuid,
            responder,
            done,
        } => {
            trace!("Handling register message message: uuid={uuid}, responder={responder:#?}");
            let entry = ResponseQueueEntry {
                responder,
                expiration: Instant::now() + DEFAULT_TIMEOUT,
            };
            if queue.insert(uuid, entry).is_some() {
                warn!("collision detected in responders queue");
            }

            if done.send(()).is_err() {
                warn!("done channel is closed");
            }
        }
        QueuedResponseMessage::Respond(ResponseMessage::Ucs(response)) => {
            handle_response_ucs_response(
                &response,
                queue,
                responses_states,
                registered_to_ucs,
                continuation_sender,
            );
        }

        QueuedResponseMessage::UcsRegistration { message_id } => {
            trace!("Received message to register to UCS");
            responses_states.registration_data = Some(RegistrationData {
                message_id,
                instant: Instant::now(),
            });
        }

        QueuedResponseMessage::UcsAccessRequest { request, responder } => {
            let access_requests = &mut responses_states.access_requests;
            match access_requests
                .binary_search_by_key(&request.request_uuid, |request| request.request_uuid)
            {
                Ok(index) => access_requests[index] = request,
                Err(index) => access_requests.insert(index, request),
            }
            if responder.send(()).is_err() {
                warn!("unable to respond to task to notify the ucs access request has been inserted/updated");
            }
        }

        QueuedResponseMessage::Respond(ResponseMessage::Internal(InternalResponseMessage {
            request_id,
            body,
            message_type: _,
        })) => {
            trace!("Handling respond message message: request_id={request_id}, body={body:#?}");
            if let Some(entry) = queue.remove(&request_id) {
                debug!("Dequeued entry from queue: {entry:#?}");

                match entry.responder {
                    Responder::Bool(sender) => {
                        deserialize_and_respond(request_id, body, sender, Ok);
                    }
                    Responder::U8(sender) => {
                        deserialize_and_respond(request_id, body, sender, Ok);
                    }
                    Responder::Unit(sender) => {
                        deserialize_and_respond(request_id, body, sender, Ok);
                    }
                    Responder::Thing {
                        sender,
                        registration,
                    } => deserialize_and_respond(request_id, body, sender, |thing| {
                        Continuation::Thing {
                            thing,
                            ty: registration,
                        }
                    }),
                }
            } else {
                warn!("request {request_id} not found in registered responders");
            }
        }
    }

    let now = Instant::now();
    if *last_cleanup + DEFAULT_TIMEOUT <= now {
        queue.retain(|_, entry| entry.expiration > now);
        *last_cleanup = now;
    }
}

fn handle_response_ucs_response(
    response: &ucs::Response,
    queue: &mut HashMap<Uuid, ResponseQueueEntry>,
    responses_states: &mut ResponseStates,
    registered_to_ucs: &AtomicBool,
    continuation_sender: &mpsc::UnboundedSender<Continuation>,
) {
    let message = &response.command.value.message;
    match message.inner {
        ucs::ResponseMessageInner::RegisterResponse { code } => {
            if matches!(
                responses_states.registration_data,
                Some(RegistrationData { message_id, .. })
                if message_id == message.message_id
            ) {
                if let ucs::Code::Ok = code {
                    registered_to_ucs.store(true, atomic::Ordering::Release);
                    responses_states.registration_data = None;
                } else {
                    panic!("Unable to register to UCS");
                }
            }
        }
        ucs::ResponseMessageInner::TryResponse {
            evaluation,
            session_id,
        } => {
            handle_ucs_response_inner(
                responses_states,
                response,
                evaluation,
                continuation_sender,
                queue,
                |access_request| {
                    Some(Continuation::ToUcs(continuation::ToUcs {
                        session_id,
                        request_uuid: access_request.request_uuid,
                        kind: continuation::ToUcsKind::StartResponse,
                        thing_type: access_request.thing_type,
                    }))
                },
            );
        }
        ucs::ResponseMessageInner::StartResponse { evaluation } => {
            handle_ucs_response_inner(
                responses_states,
                response,
                evaluation,
                continuation_sender,
                queue,
                |access_request| {
                    let AccessRequestKind::Start { session_id } = access_request.kind else {
                        warn!(
                            "Access request is expected to be a start kind, found {:?}",
                            access_request.kind,
                        );
                        return None;
                    };

                    Some(Continuation::ToUcs(continuation::ToUcs {
                        session_id,
                        request_uuid: access_request.request_uuid,
                        kind: continuation::ToUcsKind::EndResponse,
                        thing_type: access_request.thing_type,
                    }))
                },
            );
        }

        ucs::ResponseMessageInner::EndResponse { evaluation } => {
            handle_ucs_response_inner(
                responses_states,
                response,
                evaluation,
                continuation_sender,
                queue,
                |access_request| {
                    let AccessRequest {
                        request_uuid,
                        message_id: _,
                        kind,
                        thing_type,
                    } = access_request;

                    if matches!(kind, AccessRequestKind::End { .. }).not() {
                        warn!(
                            "Access request is expected to be a end kind, found {:?}",
                            kind,
                        );
                        return None;
                    }

                    Some(Continuation::Interact {
                        request_uuid,
                        ty: thing_type,
                    })
                },
            );
        }

        ucs::ResponseMessageInner::ReevaluationResponse {
            evaluation,
            session_id,
        } => {
            info!("Ignoring reevaluation response from UCS (evaluation: {evaluation}, session_id: {session_id})");
        }
    }
}

trait SenderChannel<T> {
    type Err;

    fn send(self, value: T) -> Result<(), Self::Err>;
}

impl<T> SenderChannel<T> for oneshot::Sender<T> {
    type Err = T;

    fn send(self, value: T) -> Result<(), Self::Err> {
        self.send(value)
    }
}

impl<T> SenderChannel<T> for mpsc::UnboundedSender<T> {
    type Err = mpsc::error::SendError<T>;

    fn send(self, value: T) -> Result<(), Self::Err> {
        mpsc::UnboundedSender::send(&self, value)
    }
}

fn deserialize_and_respond<T, U, F>(
    request_id: Uuid,
    body: Option<serde_json::Value>,
    sender: impl SenderChannel<U>,
    mapper: F,
) where
    T: DeserializeOwned,
    F: FnOnce(T) -> U,
{
    let body = body.unwrap_or(serde_json::Value::Null);
    let response = match serde_json::from_value(body) {
        Ok(x) => x,
        Err(err) => {
            error!(
                "cannot convert JSON response to the required {} for request {request_id}: {err}",
                any::type_name::<T>(),
            );
            return;
        }
    };

    if sender.send(mapper(response)).is_err() {
        warn!("cannot respond to message for request {request_id}, channel is closed");
    }
}

#[derive(Debug)]
enum QueuedResponseMessage {
    Register {
        uuid: Uuid,
        responder: Responder,
        done: oneshot::Sender<()>,
    },
    Respond(ResponseMessage),
    UcsRegistration {
        message_id: Uuid,
    },
    UcsAccessRequest {
        request: AccessRequest,
        responder: oneshot::Sender<()>,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ResponseMessage {
    Internal(InternalResponseMessage),
    Ucs(ucs::Response<'static>),
}

#[derive(Debug)]
enum Responder {
    Bool(ResponseSender<bool>),
    U8(ResponseSender<u8>),
    Unit(ResponseSender<()>),
    Thing {
        sender: mpsc::UnboundedSender<Continuation>,
        registration: registration::ThingType,
    },
}

impl Responder {
    pub fn respond_with_error(self) -> Result<(), ChannelClosedError> {
        match self {
            Responder::Bool(sender) => sender
                .send(Err(ResponseError))
                .map_err(|_| ChannelClosedError),
            Responder::U8(sender) => sender
                .send(Err(ResponseError))
                .map_err(|_| ChannelClosedError),
            Responder::Unit(sender) => sender
                .send(Err(ResponseError))
                .map_err(|_| ChannelClosedError),
            Responder::Thing { .. } => {
                panic!("cannot use respond_with_error on a Responder::Thing variant")
            }
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ChannelClosedError;

impl Display for ChannelClosedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("channel closed")
    }
}

impl std::error::Error for ChannelClosedError {}

#[derive(Debug, Default)]
struct ResponseStates {
    registration_data: Option<RegistrationData>,
    access_requests: Vec<AccessRequest>,
}

#[derive(Debug)]
struct RegistrationData {
    message_id: Uuid,
    instant: Instant,
}

#[derive(Debug)]
struct AccessRequest {
    request_uuid: Uuid,
    message_id: Uuid,
    kind: AccessRequestKind,
    thing_type: registration::ThingType,
}

#[derive(Debug)]
enum AccessRequestKind {
    Try,
    Start { session_id: Uuid },
    End,
}

fn handle_ucs_response_inner<F>(
    responses_states: &mut ResponseStates,
    response: &ucs::Response,
    evaluation: ucs::Evaluation,
    continuation_sender: &mpsc::UnboundedSender<Continuation>,
    queue: &mut HashMap<Uuid, ResponseQueueEntry>,
    on_pemit: F,
) where
    F: FnOnce(AccessRequest) -> Option<Continuation>,
{
    let message = &response.command.value.message;
    let Some(access_request_index) = responses_states
        .access_requests
        .iter()
        .position(|request| request.message_id == message.message_id)
    else {
        warn!("Unable to find request message id {}", message.message_id);
        return;
    };

    let access_request = responses_states
        .access_requests
        .remove(access_request_index);

    match evaluation {
        ucs::Evaluation::Permit => {
            let Some(continuation_message) = on_pemit(access_request) else {
                error!("Unable to perform continuation on permit evaluation");
                return;
            };

            if continuation_sender.send(continuation_message).is_err() {
                error!("Unable to send notify task to send a start response message to the UCS");
            }
        }
        ucs::Evaluation::Deny => {
            let Some(request) = queue.remove(&access_request.request_uuid) else {
                warn!("Unable to find request in queue to respond with a deny");
                return;
            };

            if request.responder.respond_with_error().is_err() {
                warn!("Unable to respond with deny: channel closed");
            }
        }
    }
}

async fn registration_handler(
    cache_sender: &mpsc::Sender<Registration>,
    registered_to_ucs: &AtomicBool,
    topic_name: &Arc<str>,
    topic_uuid: Uuid,
) {
    let send_registration = || async {
        trace!("Sending registration message to UCS");
        cache_sender
            .send(Registration::RegisterToUcs {
                topic_name: Arc::clone(topic_name),
                topic_uuid,
            })
            .await
            .expect("unable to send registration message, channel closed");
    };

    send_registration().await;
    loop {
        if registered_to_ucs.load(atomic::Ordering::Acquire) {
            sleep(Duration::from_secs(60 * 5)).await;
        } else {
            sleep(Duration::from_secs(10)).await;
        }

        send_registration().await;
    }
}
