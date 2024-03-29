use std::{
    future::{Future, IntoFuture},
    pin::Pin,
    sync::Arc,
    task::{self, Poll},
    time::Duration,
};

use log::info;
use pin_project_lite::pin_project;
use sifis_message::RequestMessage;
use tokio::{
    sync::oneshot,
    time::{self, Timeout},
};
use uuid::Uuid;

use crate::{continuation, PeerId, DEFAULT_TIMEOUT};

#[derive(Debug)]
pub(super) enum Registration {
    Thing {
        request_uuid: Uuid,
        ty: ThingType,
        operation: ThingOperation,
    },
    Raw(RequestMessage),
    GetTopicName {
        name: String,
        responder: oneshot::Sender<Result<serde_json::Value, String>>,
    },
    RegisterToUcs {
        topic_name: Arc<str>,
        topic_uuid: Uuid,
    },
    ToUcs(continuation::ToUcs),
}

#[derive(Debug)]
pub struct ThingType {
    pub thing_uuid: Uuid,
    pub registration: ThingTypeRegistration,
}

// TODO: find a better name and remove the lint
#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub enum ThingTypeRegistration {
    Lamp(Lamp),
    Sink(Sink),
}

#[derive(Debug)]
pub enum Lamp {
    OnOff(ResponseSender<bool>),
    Brightness(ResponseSender<u8>),
    SetOn {
        value: bool,
        responder: ResponseSender<()>,
    },
    SetBrightness {
        value: u8,
        responder: ResponseSender<()>,
    },
}

#[derive(Debug)]
pub enum Sink {
    Flow(ResponseSender<u8>),
    Temp(ResponseSender<u8>),
    Level(ResponseSender<u8>),
    SetFlow {
        value: u8,
        responder: ResponseSender<()>,
    },
    SetTemp {
        value: u8,
        responder: ResponseSender<()>,
    },
    SetDrain {
        open: bool,
        responder: ResponseSender<()>,
    },
}

pub type ResponseSender<T> = oneshot::Sender<Result<T, ResponseError>>;

// TODO: use an enum
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResponseError;

#[derive(Debug)]
pub(super) enum ThingOperation {
    RequestPermission(PeerId),
    Interact,
}

impl Registration {
    pub(super) fn get_topic_name(
        name: impl Into<String>,
    ) -> (Self, Receiver<Result<serde_json::Value, String>>) {
        let name = name.into();
        info!(r#"get_topic_name run with name "{name}""#);
        Self::build(|_request_uuid, responder| Self::GetTopicName { name, responder })
    }

    pub(super) fn lamp_on_off(thing_uuid: Uuid, peer_id: PeerId) -> (Self, ResponseReceiver<bool>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Lamp(Lamp::OnOff(responder)),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn lamp_brightness(
        thing_uuid: Uuid,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<u8>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Lamp(Lamp::Brightness(responder)),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn lamp_set_on(
        thing_uuid: Uuid,
        value: bool,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<()>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Lamp(Lamp::SetOn { value, responder }),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn lamp_set_brightness(
        thing_uuid: Uuid,
        value: u8,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<()>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Lamp(Lamp::SetBrightness { value, responder }),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_flow(thing_uuid: Uuid, peer_id: PeerId) -> (Self, ResponseReceiver<u8>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::Flow(responder)),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_temp(thing_uuid: Uuid, peer_id: PeerId) -> (Self, ResponseReceiver<u8>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::Temp(responder)),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_level(topic_uuid: Uuid, peer_id: PeerId) -> (Self, ResponseReceiver<u8>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::Level(responder)),
                thing_uuid: topic_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_set_flow(
        thing_uuid: Uuid,
        value: u8,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<()>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::SetFlow { value, responder }),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_set_temp(
        thing_uuid: Uuid,
        value: u8,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<()>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::SetTemp { value, responder }),
                thing_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn sink_set_drain(
        topic_uuid: Uuid,
        open: bool,
        peer_id: PeerId,
    ) -> (Self, ResponseReceiver<()>) {
        Self::build(|request_uuid, responder| Self::Thing {
            request_uuid,
            ty: ThingType {
                registration: ThingTypeRegistration::Sink(Sink::SetDrain { open, responder }),
                thing_uuid: topic_uuid,
            },
            operation: ThingOperation::RequestPermission(peer_id),
        })
    }

    pub(super) fn build<T, F>(f: F) -> (Self, Receiver<T>)
    where
        F: FnOnce(Uuid, oneshot::Sender<T>) -> Self,
    {
        let (sender, receiver) = oneshot::channel();
        let request_uuid = Uuid::new_v4();
        let receiver = Receiver::new(receiver);

        (f(request_uuid, sender), receiver)
    }
}

#[derive(Debug)]
pub struct Receiver<T> {
    receiver: oneshot::Receiver<T>,
    timeout: Duration,
}

impl<T> Receiver<T> {
    fn new(receiver: oneshot::Receiver<T>) -> Self {
        Self {
            receiver,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

type ResponseReceiver<T> = Receiver<Result<T, ResponseError>>;

pin_project! {
    pub struct ReceiverFuture<T>{
        #[pin]
        inner: Timeout<oneshot::Receiver<T>>,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiverError {
    Timeout,
    Recv,
}

impl<T> Future for ReceiverFuture<T> {
    type Output = Result<T, ReceiverError>;

    fn poll(self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
        let inner = self.project().inner;
        let out = task::ready!(inner.poll(cx));

        Poll::Ready(match out {
            Ok(Ok(x)) => Ok(x),
            Ok(Err(_)) => Err(ReceiverError::Recv),
            Err(_) => Err(ReceiverError::Timeout),
        })
    }
}

impl<T> IntoFuture for Receiver<T> {
    type Output = Result<T, ReceiverError>;
    type IntoFuture = ReceiverFuture<T>;

    fn into_future(self) -> Self::IntoFuture {
        let Self { receiver, timeout } = self;
        ReceiverFuture {
            inner: time::timeout(timeout, receiver),
        }
    }
}
