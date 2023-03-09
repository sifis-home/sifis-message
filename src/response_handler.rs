use std::{
    future::{Future, IntoFuture},
    ops::Not,
    pin::Pin,
    task::{self, Poll},
    time::Duration,
};

use log::warn;
use pin_project_lite::pin_project;
use serde::Deserialize;
use sifis_api::service::Error as SifisError;
use sifis_dht::domocache::DomoEvent;
use tokio::{
    sync::{mpsc, oneshot},
    time::{Instant, Sleep},
};

use crate::VolatileMessage;

#[derive(Debug, Clone)]
pub struct Sender {
    request_sender: mpsc::Sender<Registration>,
}

#[derive(Debug)]
pub struct Receiver {
    request_receiver: mpsc::Receiver<Registration>,
    domo_event_receiver: mpsc::Receiver<DomoEvent>,
    queued: Vec<QueuedRegistration>,
    response_timeout: Duration,
}

pub(crate) type DomoEventSender = mpsc::Sender<DomoEvent>;

#[derive(Debug)]
struct Registration {
    ty: RegistrationType,
    uuid: String,
}

#[derive(Debug)]
enum RegistrationType {
    LampOnOff(ResponseSender<bool>),
    LampBrightness(ResponseSender<u8>),
    SinkFlow(ResponseSender<u8>),
    SinkTemp(ResponseSender<u8>),
    SinkDrain(ResponseSender<bool>),
    SinkLevel(ResponseSender<u8>),
}

type Response<T> = Result<T, SifisError>;

type ResponseSender<T> = oneshot::Sender<Response<T>>;

#[derive(Debug)]
struct QueuedRegistration {
    registration: Registration,
    expiration: Instant,
}

impl Sender {
    #[inline]
    pub async fn lamp_on_off(&self, uuid: impl Into<String>) -> Result<bool, SifisError> {
        self.send_and_get_response(uuid, RegistrationType::LampOnOff)
            .await
    }

    #[inline]
    pub async fn lamp_brightness(&self, uuid: impl Into<String>) -> Result<u8, SifisError> {
        self.send_and_get_response(uuid, RegistrationType::LampBrightness)
            .await
    }

    #[inline]
    pub async fn sink_flow(&self, uuid: impl Into<String>) -> Result<u8, SifisError> {
        self.send_and_get_response(uuid, RegistrationType::SinkFlow)
            .await
    }

    #[inline]
    pub async fn sink_drain(&self, uuid: impl Into<String>) -> Result<bool, SifisError> {
        self.send_and_get_response(uuid, RegistrationType::SinkDrain)
            .await
    }

    #[inline]
    pub async fn sink_level(&self, uuid: impl Into<String>) -> Result<u8, SifisError> {
        self.send_and_get_response(uuid, RegistrationType::SinkLevel)
            .await
    }

    async fn send_and_get_response<F, T>(
        &self,
        uuid: impl Into<String>,
        f: F,
    ) -> Result<T, SifisError>
    where
        F: FnOnce(ResponseSender<T>) -> RegistrationType,
    {
        let (sender, receiver) = oneshot::channel();
        let ty = f(sender);
        let uuid = uuid.into();

        self.request_sender
            .send(Registration { ty, uuid })
            .await
            .expect("Response handler channel should be open");

        receiver
            .await
            .expect("Response handler channel should be open")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "thingType")]
struct VolatileForeignMessage {
    uuid: String,
    data: ForeignMessageData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ForeignMessageData {
    Lamp(LampForeignMessageData),
    Sink(SinkForeignMessageData),
}

type ForeignResponse<T> = Result<T, String>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum LampForeignMessageData {
    OnOff(ForeignResponse<bool>),
    Brightness(ForeignResponse<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
enum SinkForeignMessageData {
    Flow(ForeignResponse<u8>),
    Temp(ForeignResponse<u8>),
    Drain(ForeignResponse<bool>),
    Level(ForeignResponse<u8>),
}

pin_project! {
    #[project = ReceiverFutureProj]
    pub struct ReceiverFuture {
        receiver: Receiver,
        #[pin]
        timeout: Option<Sleep>,
    }
}

impl IntoFuture for Receiver {
    type Output = Result<(), ()>;
    type IntoFuture = ReceiverFuture;

    fn into_future(self) -> Self::IntoFuture {
        ReceiverFuture {
            receiver: self,
            timeout: None,
        }
    }
}

impl Future for ReceiverFuture {
    type Output = Result<(), ()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Poll::Ready(opt) = this.receiver.domo_event_receiver.poll_recv(cx) {
            match opt {
                Some(domo_event) => this.handle_domo_event(domo_event),
                None => return Poll::Ready(Ok(())),
            }
        }

        if let Poll::Ready(opt) = this.receiver.request_receiver.poll_recv(cx) {
            match opt {
                Some(request) => todo!(),
                None => return Poll::Ready(Ok(())),
            }
        }

        if let Some(mut timeout) = this.timeout.as_pin_mut() {
            if let Poll::Ready(()) = timeout.as_mut().poll(cx) {
                todo!()
            }
        }

        Poll::Pending
    }
}

impl ReceiverFutureProj<'_> {
    fn handle_domo_event(&mut self, domo_event: DomoEvent) {
        let DomoEvent::VolatileData(data) = domo_event else {
            return;
        };

        let Ok(message) = serde_json::from_value::<VolatileForeignMessage>(data) else {
            return;
        };

        let operation_result = self
            .receiver
            .queued
            .iter()
            .enumerate()
            .filter(|(_, entry)| message.uuid == entry.registration.uuid)
            .find_map(|(index, entry)| {
                let operation_result = match (message.data, &entry.registration.ty) {
                    (
                        ForeignMessageData::Lamp(LampForeignMessageData::OnOff(value)),
                        RegistrationType::LampOnOff(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    (
                        ForeignMessageData::Lamp(LampForeignMessageData::Brightness(value)),
                        RegistrationType::LampBrightness(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    (
                        ForeignMessageData::Sink(SinkForeignMessageData::Flow(value)),
                        RegistrationType::SinkFlow(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    (
                        ForeignMessageData::Sink(SinkForeignMessageData::Temp(value)),
                        RegistrationType::SinkTemp(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    (
                        ForeignMessageData::Sink(SinkForeignMessageData::Drain(value)),
                        RegistrationType::SinkDrain(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    (
                        ForeignMessageData::Sink(SinkForeignMessageData::Level(value)),
                        RegistrationType::SinkLevel(sender),
                    ) => Some(sender.send(value.map_err(SifisError::Forbidden)).is_ok()),
                    _ => None,
                };

                operation_result.map(|opt| (index, opt))
            });

        if let Some((index, channel_sent)) = operation_result {
            if channel_sent.not() {
                warn!("Unable to respond to uuid {}", message.uuid);
            }

            self.receiver.queued.remove(index);
        }
    }
}
