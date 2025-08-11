mod handler;

use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
pub use handler::Handler;

use parking_lot::{Mutex, MutexGuard};

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    convert::Infallible,
    io,
    sync::Arc,
    task::{Context, Poll},
};

use volans_core::{PeerId, Url};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, PeerCondition, StreamProtocol, StreamUpgradeError, SubstreamProtocol,
    THandlerAction, THandlerEvent,
    error::{ConnectionError, DialError},
};

use crate::OutboundStreamUpgradeFactory;
pub struct Behavior<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    shared: Arc<Mutex<Shared<TFactory>>>,
    dial_receiver: mpsc::Receiver<PeerId>,
}

impl<TFactory> Behavior<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    pub fn new(factory: TFactory) -> Self {
        let (dial_sender, dial_receiver) = mpsc::channel(0);
        let shared = Arc::new(Mutex::new(Shared::new(factory, dial_sender)));
        Self {
            shared,
            dial_receiver,
        }
    }

    pub fn controller(&self) -> Controller<TFactory> {
        Controller::new(self.shared.clone())
    }
}

impl<TFactory> NetworkBehavior for Behavior<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    type ConnectionHandler = Handler<TFactory>;
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

impl<TFactory> NetworkOutgoingBehavior for Behavior<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(Handler::new(
            self.shared.clone(),
            Shared::lock(&self.shared).receiver(peer_id, id),
        ))
    }

    fn on_connection_established(&mut self, id: ConnectionId, peer_id: PeerId, _addr: &Url) {
        Shared::lock(&self.shared).on_connection_established(peer_id, id);
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _addr: &Url,
        _reason: Option<&ConnectionError>,
    ) {
        Shared::lock(&self.shared).on_connection_closed(peer_id, id);
    }

    fn on_dial_failure(
        &mut self,
        _id: ConnectionId,
        peer_id: Option<PeerId>,
        _addr: Option<&Url>,
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

pub(crate) struct Shared<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    upgrade_factory: TFactory,
    connections: HashMap<PeerId, HashSet<ConnectionId>>,
    senders: HashMap<ConnectionId, mpsc::Sender<handler::NewOutboundStream<TFactory>>>,
    pending_channels: HashMap<
        PeerId,
        (
            mpsc::Sender<handler::NewOutboundStream<TFactory>>,
            mpsc::Receiver<handler::NewOutboundStream<TFactory>>,
        ),
    >,
    dial_sender: mpsc::Sender<PeerId>,
}

impl<TFactory> Shared<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    fn new(upgrade_factory: TFactory, dial_sender: mpsc::Sender<PeerId>) -> Self {
        Self {
            upgrade_factory,
            connections: HashMap::new(),
            senders: HashMap::new(),
            pending_channels: HashMap::new(),
            dial_sender,
        }
    }

    fn lock(shared: &Arc<Mutex<Self>>) -> MutexGuard<'_, Self> {
        shared.lock()
    }

    fn on_connection_established(&mut self, peer_id: PeerId, conn_id: ConnectionId) {
        self.connections.entry(peer_id).or_default().insert(conn_id);
    }

    fn on_connection_closed(&mut self, peer_id: PeerId, conn_id: ConnectionId) {
        match self.connections.entry(peer_id.clone()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().remove(&conn_id);
                if entry.get().is_empty() {
                    entry.remove();
                }
            }
            Entry::Vacant(_) => {}
        }
    }

    fn on_dial_failure(&mut self, peer_id: PeerId, error: &DialError) {
        let Some((_, mut receiver)) = self.pending_channels.remove(&peer_id) else {
            return;
        };
        while let Ok(Some(request)) = receiver.try_next() {
            let _ = request
                .sender
                .send(Err(StreamUpgradeError::Io(io::Error::new(
                    io::ErrorKind::NotConnected,
                    error.to_string(),
                ))));
        }
    }

    fn sender(&mut self, peer: PeerId) -> mpsc::Sender<handler::NewOutboundStream<TFactory>> {
        let maybe_sender = self
            .connections
            .get_mut(&peer)
            .and_then(|conns| conns.iter().next())
            .and_then(|i| self.senders.get(i));

        match maybe_sender {
            Some(sender) => sender.clone(),
            None => {
                let (sender, _) = self
                    .pending_channels
                    .entry(peer)
                    .or_insert_with(|| mpsc::channel(0));
                let _ = self.dial_sender.try_send(peer);
                sender.clone()
            }
        }
    }

    fn receiver(
        &mut self,
        peer: PeerId,
        connection_id: ConnectionId,
    ) -> mpsc::Receiver<handler::NewOutboundStream<TFactory>> {
        if let Some((sender, receiver)) = self.pending_channels.remove(&peer) {
            tracing::debug!(%peer, %connection_id, "Returning existing pending receiver");
            self.senders.insert(connection_id, sender);
            return receiver;
        }
        tracing::debug!(%peer, %connection_id, "Creating new channel pair");
        let (sender, receiver) = mpsc::channel(0);
        self.senders.insert(connection_id, sender);
        receiver
    }

    fn outbound_request(
        &self,
        protocol: StreamProtocol,
    ) -> SubstreamProtocol<TFactory::Upgrade, ()> {
        self.upgrade_factory.outbound_request(protocol)
    }
}

#[derive(Clone)]
pub struct Controller<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    shared: Arc<Mutex<Shared<TFactory>>>,
}

impl<TFactory> Controller<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    fn new(shared: Arc<Mutex<Shared<TFactory>>>) -> Self {
        Self { shared }
    }

    pub async fn open(
        &mut self,
        peer_id: PeerId,
        protocol: StreamProtocol,
    ) -> Result<TFactory::Output, StreamUpgradeError<TFactory::Error>> {
        let mut new_stream_sender = Shared::lock(&self.shared).sender(peer_id);
        let (sender, receiver) = oneshot::channel();
        new_stream_sender
            .send(handler::NewOutboundStream { protocol, sender })
            .await
            .map_err(|e| {
                StreamUpgradeError::Io(io::Error::new(io::ErrorKind::ConnectionReset, e))
            })?;

        let result = receiver.await.map_err(|e| {
            StreamUpgradeError::Io(io::Error::new(io::ErrorKind::ConnectionReset, e))
        })?;

        result
    }
}
