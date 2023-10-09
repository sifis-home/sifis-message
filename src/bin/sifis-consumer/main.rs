#![warn(clippy::pedantic)]

use std::{cell::OnceCell, ops::Not, sync::Arc, time::Duration};

use anyhow::{anyhow, bail, Context};
use clap::Parser;
use cli::{Cli, ThingType};
use futures_concurrency::future::TryJoin;
use futures_util::{stream::FuturesUnordered, FutureExt, TryStreamExt};
use log::{debug, info, trace, warn};
use reqwest::{Client, Url};
use sifis_dht::domocache::{DomoCache, DomoEvent};
use sifis_message::{
    domo_cache_from_config, manage_domo_cache, Authorization, InternalResponseMessage,
    RequestMessage, ResponseMessageType, LAMP_TOPIC_NAME, SINK_TOPIC_NAME,
};
use tokio::sync::mpsc;
use uuid::Uuid;

mod cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let domo_cache = domo_cache_from_config(&cli.cache_config_file).await?;
    info!("Created domo cache from config");

    let (things_with_uuid, mut children) =
        run_things(&cli.things).context("unable to run things")?;
    info!("Created children");

    let children_spawns = children
        .iter_mut()
        .map(|child| async move {
            let exit_code = child
                .wait()
                .await
                .context("unable to wait for child completion")?;

            if exit_code.success() {
                Ok(())
            } else {
                match child.id() {
                    Some(id) => bail!("child {id} exited with status {}", exit_code),
                    None => bail!("child with no id exited with status {}", exit_code),
                }
            }
        })
        .collect::<FuturesUnordered<_>>()
        .try_collect::<Vec<()>>();

    info!("Created children waiting futures");

    let things_with_uuid = Arc::new(things_with_uuid);

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let (message_sender, message_receiver) = mpsc::channel(32);

    if let Err(err) = (
        handle_things(
            Arc::clone(&things_with_uuid),
            domo_cache,
            client.clone(),
            message_sender.clone(),
            message_receiver,
        ),
        children_spawns,
        register_things(&things_with_uuid, message_sender).map(Ok),
    )
        .try_join()
        .await
    {
        for mut child in children {
            // Try to kill a child if still alive
            let _ = child.kill().await;
        }

        return Err(err);
    }

    Ok(())
}

#[derive(Debug)]
struct ThingRepr {
    port: u16,
    uuid: Uuid,
    ty: ThingType,
}

fn run_things(
    cli_things: &[cli::Thing],
) -> anyhow::Result<(Vec<ThingRepr>, Vec<tokio::process::Child>)> {
    let mut things_with_uuid = Vec::new();
    let mut children = Vec::new();

    for cli_thing in cli_things {
        let &cli::Thing {
            ref path,
            port,
            ty,
            uuid,
            ref args,
        } = cli_thing;
        let child = tokio::process::Command::new(path)
            .arg("--listen-port")
            .arg(format!("{port}"))
            .args(args)
            .spawn()?;
        let thing = ThingRepr { port, uuid, ty };

        things_with_uuid.push(thing);
        children.push(child);
    }

    Ok((things_with_uuid, children))
}

#[derive(Debug)]
enum Message {
    Request(RequestMessage),
    RegisterThing { topic: String, uuid: Uuid },
}

async fn handle_things(
    things_with_uuid: Arc<Vec<ThingRepr>>,
    domo_cache: DomoCache,
    client: Client,
    message_sender: mpsc::Sender<Message>,
    message_receiver: mpsc::Receiver<Message>,
) -> anyhow::Result<()> {
    Box::pin(manage_domo_cache(
        domo_cache,
        message_receiver,
        move |message, domo_cache| {
            handle_message(
                message,
                domo_cache,
                Arc::clone(&things_with_uuid),
                client.clone(),
            )
            .boxed()
        },
    ))
    .map_err(|err| anyhow!("error while managing domo cache: {err}"))
    .try_for_each(|event| async {
        let DomoEvent::VolatileData(volatile_data) = event else {
            return Ok(());
        };

        let Ok(request_message) = serde_json::from_value::<RequestMessage>(volatile_data) else {
            return Ok(());
        };

        if message_sender
            .send(Message::Request(request_message))
            .await
            .is_err()
        {
            warn!("message receiver for domo cache is unexpectedly closed");
        }
        Ok(())
    })
    .await?;

    Ok(())
}

async fn handle_message(
    message: Message,
    domo_cache: &mut DomoCache,
    things_with_uuid: Arc<Vec<ThingRepr>>,
    client: Client,
) -> anyhow::Result<()> {
    trace!("Handling message: {message:#?}");
    match message {
        Message::Request(request_message) => {
            handle_request_message(request_message, domo_cache, things_with_uuid, client).await
        }
        Message::RegisterThing { topic, uuid } => {
            info!("Registering {uuid} on topic {topic}");
            domo_cache
                .write_value(&topic, &uuid.to_string(), serde_json::Value::Null)
                .await;
            Ok(())
        }
    }
}

async fn handle_request_message(
    message: RequestMessage,
    domo_cache: &mut DomoCache,
    things_with_uuid: Arc<Vec<ThingRepr>>,
    client: Client,
) -> anyhow::Result<()> {
    thread_local! {
        static LOCALHOST: OnceCell<Url> = const { OnceCell::new() };
    }

    let RequestMessage {
        message_type: _,
        request_id,
        thing_id,
        method,
        path,
        authorization,
        headers,
        body,
    } = message;

    debug!("Trying to handle message for thing {thing_id}");
    let Some(port) = things_with_uuid
        .iter()
        .find_map(|thing| (thing.uuid == thing_id).then_some(thing.port))
    else {
        debug!("Thing {thing_id} is not available");
        return Ok(());
    };

    let mut url = LOCALHOST.with(|localhost| {
        localhost
            .get_or_init(|| Url::parse("http://localhost").unwrap())
            .clone()
    });
    url.set_port(Some(port)).unwrap();
    url.set_path(&path);

    debug!("Performing a request on url {url} with method {method} for thing {thing_id}");
    let mut builder = client.request(method, url);
    if let Some(authorization) = authorization {
        builder = match authorization {
            Authorization::Basic { username, password } => {
                builder.basic_auth(username, Some(password))
            }
            Authorization::Bearer(token) => builder.bearer_auth(token),
        }
    }

    if body.is_empty().not() {
        builder = builder.body(body);
    }

    if headers.is_empty().not() {
        builder = builder.headers(headers.into_iter().collect());
    }

    let response = builder
        .send()
        .await
        .with_context(|| format!("unable to send request to port {port}"))?
        .error_for_status()
        .with_context(|| format!("thing on port {port} returned an error code"))?;

    let body: Option<serde_json::Value> =
        if matches!(response.content_length(), Some(len) if len > 0) {
            let data = response.json().await.with_context(|| {
                format!("cannot obtain json response from thing on port {port}")
            })?;

            Some(data)
        } else {
            None
        };

    let response = InternalResponseMessage {
        request_id,
        body,
        message_type: ResponseMessageType::ResponseMessage,
    };

    trace!("Sending response: {response:#?}");
    domo_cache
        .pub_value(
            serde_json::to_value(response).context("unable to convert response body to json")?,
        )
        .await;

    Ok(())
}

async fn register_things(things_with_uuid: &[ThingRepr], message_sender: mpsc::Sender<Message>) {
    tokio::time::sleep(Duration::from_secs(5)).await;
    for &ThingRepr { uuid, ty, .. } in things_with_uuid {
        let topic = match ty {
            ThingType::Lamp => LAMP_TOPIC_NAME,
            ThingType::Sink => SINK_TOPIC_NAME,
        };

        message_sender
            .send(Message::RegisterThing {
                topic: topic.to_owned(),
                uuid,
            })
            .await
            .unwrap();
    }
}
