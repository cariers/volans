use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt, channel::mpsc};
use parking_lot::Mutex;
use volans_core::PeerId;
use volans_swarm::{ConnectionId, StreamProtocol, Substream};

mod behavior;
mod handler;
mod shared;

pub use behavior::Behavior;
pub use handler::Handler;

#[derive(Debug, thiserror::Error)]
#[error("The protocol is already registered")]
pub struct AlreadyRegistered;

pub struct IncomingStreams {
    receiver: mpsc::Receiver<(PeerId, ConnectionId, Substream)>,
}

impl IncomingStreams {
    pub(crate) fn new(receiver: mpsc::Receiver<(PeerId, ConnectionId, Substream)>) -> Self {
        Self { receiver }
    }
}

impl Stream for IncomingStreams {
    type Item = (PeerId, ConnectionId, Substream);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_next_unpin(cx)
    }
}

#[derive(Clone)]
pub struct Acceptor {
    shared: Arc<Mutex<shared::Shared>>,
}

impl Acceptor {
    fn new(shared: Arc<Mutex<shared::Shared>>) -> Self {
        Self { shared }
    }

    pub fn accept(
        &mut self,
        protocol: StreamProtocol,
    ) -> Result<IncomingStreams, AlreadyRegistered> {
        shared::Shared::lock(&self.shared).accept(protocol)
    }
}
