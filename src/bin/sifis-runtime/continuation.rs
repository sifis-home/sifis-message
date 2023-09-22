use anyhow::{bail, Context};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    registration::{self, Registration},
    AccessRequestKind,
};

#[derive(Debug)]
pub enum Continuation {
    ToUcs(ToUcs),
    Interact {
        request_uuid: Uuid,
        ty: registration::ThingType,
    },
}

#[derive(Debug)]
pub struct ToUcs {
    pub session_id: Uuid,
    pub request_uuid: Uuid,
    pub kind: ToUcsKind,
    pub thing_type: registration::ThingType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToUcsKind {
    StartResponse,
    EndResponse,
}

impl ToUcs {
    pub(super) fn access_request_kind(&self) -> AccessRequestKind {
        let &Self {
            session_id, kind, ..
        } = self;
        match kind {
            ToUcsKind::StartResponse => AccessRequestKind::Start { session_id },
            ToUcsKind::EndResponse => AccessRequestKind::End,
        }
    }
}

pub(super) async fn handle_receiver(
    mut continuation_receiver: mpsc::UnboundedReceiver<Continuation>,
    cache_sender: &mpsc::Sender<Registration>,
) -> anyhow::Result<()> {
    while let Some(continuation) = continuation_receiver.recv().await {
        match continuation {
            Continuation::ToUcs(to_ucs) => cache_sender
                .send(Registration::ToUcs(to_ucs))
                .await
                .context("unable to queue message to be sent to the UCS")?,

            Continuation::Interact { request_uuid, ty } => cache_sender
                .send(Registration::Thing {
                    request_uuid,
                    ty,
                    operation: registration::ThingOperation::Interact,
                })
                .await
                .context("unable to queue message to interact with thing")?,
        }
    }

    bail!("continuation channel is closed");
}
