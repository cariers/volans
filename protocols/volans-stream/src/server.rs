mod handle;

pub use handle::Handler;
use volans_codec::asynchronous_codec::{Decoder, Encoder};

use std::{
    collections::{HashMap, hash_map::Entry},
    convert::Infallible,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt, channel::mpsc};
use parking_lot::{Mutex, MutexGuard};
use volans_core::{PeerId, Url};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, StreamProtocol, SubstreamProtocol, THandlerAction, THandlerEvent,
    error::{ConnectionError, ListenError},
};

use crate::{InboundStreamUpgradeFactory, StreamEvent, upgrade::WithCodecFactory};

pub struct Behavior<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    shared: Arc<Mutex<Shared<TFactory>>>,
}

impl<TFactory> Behavior<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    pub fn new(factory: TFactory) -> Self {
        let shared = Arc::new(Mutex::new(Shared::new(factory)));
        Self { shared }
    }

    pub fn new_with_framed<TCodec>(codec: TCodec) -> Behavior<WithCodecFactory<TCodec>>
    where
        TCodec: Decoder + Encoder + Clone + Send + 'static,
    {
        let factory = WithCodecFactory::new(codec);
        Behavior::<WithCodecFactory<TCodec>>::new(factory)
    }

    pub fn acceptor(&self) -> Acceptor<TFactory> {
        Acceptor::new(self.shared.clone())
    }
}

impl<TFactory> NetworkBehavior for Behavior<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    type ConnectionHandler = Handler<TFactory>;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        self.shared
            .lock()
            .on_inbound_handler_event(peer_id, id, event);
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        Poll::Pending
    }
}

impl<TFactory> NetworkIncomingBehavior for Behavior<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Url,
        _remote_addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(Handler::new(self.shared.clone()))
    }

    /// 连接处理器事件处理
    fn on_connection_established(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Url,
        _remote_addr: &Url,
    ) {
    }

    fn on_connection_closed(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Url,
        _remote_addr: &Url,
        _reason: Option<&ConnectionError>,
    ) {
    }

    /// 监听失败事件处理
    fn on_listen_failure(
        &mut self,
        _id: ConnectionId,
        _peer_id: Option<PeerId>,
        _local_addr: &Url,
        _remote_addr: &Url,
        _error: &ListenError,
    ) {
    }

    /// 监听器事件处理
    fn on_listener_event(&mut self, _event: ListenerEvent<'_>) {}
}

pub(crate) struct Shared<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    upgrade_factory: TFactory,
    supported_protocols:
        HashMap<StreamProtocol, mpsc::Sender<(PeerId, ConnectionId, TFactory::Output)>>,
}

impl<TFactory> Shared<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    fn new(upgrade_factory: TFactory) -> Self {
        let supported_protocols = HashMap::new();
        Self {
            upgrade_factory,
            supported_protocols,
        }
    }

    fn lock(shared: &Arc<Mutex<Self>>) -> MutexGuard<'_, Self> {
        shared.lock()
    }

    fn accept(
        &mut self,
        protocol: StreamProtocol,
    ) -> Result<IncomingStreams<TFactory>, AlreadyRegistered> {
        if self.supported_protocols.contains_key(&protocol) {
            return Err(AlreadyRegistered);
        }
        let (sender, receiver) = mpsc::channel(0);
        self.supported_protocols.insert(protocol.clone(), sender);

        Ok(IncomingStreams::new(receiver))
    }

    fn listen_protocol(&mut self) -> SubstreamProtocol<TFactory::Upgrade, ()> {
        self.supported_protocols
            .retain(|_, sender| !sender.is_closed());
        let protocols = self.supported_protocols.keys().cloned().collect();
        self.upgrade_factory.listen_protocol(protocols)
    }

    fn on_inbound_handler_event(
        &mut self,
        peer_id: PeerId,
        connection_id: ConnectionId,
        event: StreamEvent<TFactory::Output, TFactory::Error>,
    ) {
        match event {
            StreamEvent::FullyNegotiated { output, protocol } => {
                match self.supported_protocols.entry(protocol.clone()) {
                    Entry::Occupied(mut entry) => {
                        match entry.get_mut().try_send((peer_id, connection_id, output)) {
                            Ok(()) => {}
                            Err(e) if e.is_full() => {
                                tracing::debug!(%protocol, "Channel is full, dropping inbound stream");
                            }
                            Err(e) if e.is_disconnected() => {
                                tracing::debug!(%protocol, "Channel is gone, dropping inbound stream");
                                entry.remove();
                            }
                            _ => unreachable!(),
                        }
                    }
                    Entry::Vacant(_) => {
                        tracing::debug!(%protocol, "channel is gone, dropping inbound stream");
                    }
                }
            }
            StreamEvent::UpgradeError { error } => {
                tracing::debug!(%peer_id, %connection_id, "Upgrade error: {:?}", error);
            }
        };
    }
}

#[derive(Debug, thiserror::Error)]
#[error("The protocol is already registered")]
pub struct AlreadyRegistered;

/// 进来的流处理器
pub struct IncomingStreams<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    receiver: mpsc::Receiver<(PeerId, ConnectionId, TFactory::Output)>,
}

impl<TFactory> IncomingStreams<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    pub(crate) fn new(receiver: mpsc::Receiver<(PeerId, ConnectionId, TFactory::Output)>) -> Self {
        Self { receiver }
    }
}

impl<TFactory> Stream for IncomingStreams<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    type Item = (PeerId, ConnectionId, TFactory::Output);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_next_unpin(cx)
    }
}

#[derive(Clone)]
pub struct Acceptor<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    shared: Arc<Mutex<Shared<TFactory>>>,
}

impl<TFactory> Acceptor<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    fn new(shared: Arc<Mutex<Shared<TFactory>>>) -> Self {
        Self { shared }
    }

    pub fn accept(
        &mut self,
        protocol: StreamProtocol,
    ) -> Result<IncomingStreams<TFactory>, AlreadyRegistered> {
        Shared::lock(&self.shared).accept(protocol)
    }
}
