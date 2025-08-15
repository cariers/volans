use std::{
    convert::Infallible,
    io,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
use parking_lot::Mutex;
use volans_core::{Multiaddr, PeerId};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, PeerCondition, StreamProtocol, Substream, THandlerAction,
    THandlerEvent,
    error::{ConnectionError, DialError},
};

use crate::client::{StreamError, handler, shared::Shared};

pub struct Behavior {
    shared: Arc<Mutex<Shared>>,
    dial_receiver: mpsc::Receiver<PeerId>,
}

impl Behavior {
    pub fn new() -> Self {
        let (dial_sender, dial_receiver) = mpsc::channel(0);
        let shared = Arc::new(Mutex::new(Shared::new(dial_sender)));
        Self {
            shared,
            dial_receiver,
        }
    }

    pub fn controller(&self) -> Controller {
        Controller::new(self.shared.clone())
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = handler::Handler;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _event: THandlerEvent<Self>,
    ) {
        unreachable!()
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        Poll::Pending
    }
}

impl NetworkOutgoingBehavior for Behavior {
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(handler::Handler::new(
            Shared::lock(&self.shared).receiver(peer_id, id),
        ))
    }

    fn on_connection_established(&mut self, id: ConnectionId, peer_id: PeerId, _addr: &Multiaddr) {
        Shared::lock(&self.shared).on_connection_established(peer_id, id);
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
        Shared::lock(&self.shared).on_connection_closed(peer_id, id);
    }

    fn on_dial_failure(
        &mut self,
        _id: ConnectionId,
        peer_id: Option<PeerId>,
        _addr: Option<&Multiaddr>,
        error: &DialError,
    ) {
        if let Some(peer_id) = peer_id {
            Shared::lock(&self.shared).on_dial_failure(peer_id, error);
        }
    }

    fn poll_dial(&mut self, cx: &mut Context<'_>) -> Poll<DialOpts> {
        if let Poll::Ready(Some(peer)) = self.dial_receiver.poll_next_unpin(cx) {
            return Poll::Ready(
                DialOpts::new(None, Some(peer))
                    .with_condition(PeerCondition::DisconnectedAndNotDialing),
            );
        }
        Poll::Pending
    }
}

#[derive(Clone)]
pub struct Controller {
    shared: Arc<Mutex<Shared>>,
}

impl Controller {
    fn new(shared: Arc<Mutex<Shared>>) -> Self {
        Self { shared }
    }

    pub async fn open(
        &mut self,
        peer_id: PeerId,
        protocol: StreamProtocol,
    ) -> Result<Substream, StreamError> {
        let mut new_stream_sender = Shared::lock(&self.shared).sender(peer_id);
        let (sender, receiver) = oneshot::channel();
        new_stream_sender
            .send(handler::NewStream { protocol, sender })
            .await
            .map_err(|e| StreamError::Io(io::Error::new(io::ErrorKind::ConnectionReset, e)))?;

        let result = receiver
            .await
            .map_err(|e| StreamError::Io(io::Error::new(io::ErrorKind::ConnectionReset, e)))?;

        result
    }
}
