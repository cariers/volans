use std::{
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use parking_lot::Mutex;
use volans_core::{PeerId, Multiaddr};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, THandlerAction, THandlerEvent,
    error::{ConnectionError, ListenError},
};

use super::{Acceptor, handler, shared::Shared};

pub struct Behavior {
    shared: Arc<Mutex<Shared>>,
}

impl Behavior {
    pub fn new() -> Self {
        let shared = Arc::new(Mutex::new(Shared::new()));
        Self { shared }
    }

    pub fn acceptor(&self) -> Acceptor {
        Acceptor::new(self.shared.clone())
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = handler::Handler;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        unreachable!("Unexpected event: {:?}", event);
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        Poll::Pending
    }
}

impl NetworkIncomingBehavior for Behavior {
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(handler::Handler::new(peer_id, id, self.shared.clone()))
    }

    /// 连接处理器事件处理
    fn on_connection_established(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) {
    }

    fn on_connection_closed(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
    }

    /// 监听失败事件处理
    fn on_listen_failure(
        &mut self,
        _id: ConnectionId,
        _peer_id: Option<PeerId>,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
        _error: &ListenError,
    ) {
    }

    /// 监听器事件处理
    fn on_listener_event(&mut self, _event: ListenerEvent<'_>) {}
}
