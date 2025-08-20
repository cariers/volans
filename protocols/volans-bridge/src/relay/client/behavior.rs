use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    task::{Context, Poll},
};

use either::Either;
use futures::{StreamExt, channel::mpsc, ready};
use volans_core::{Multiaddr, PeerId};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, THandlerAction, THandlerEvent,
    behavior::NotifyHandler,
    error::{ConnectionError, DialError},
    handler::DummyHandler,
};

use crate::{MultiaddrExt, relay::CircuitRequest};

use super::handler;

/// 中继服务器连接Backend的行为
pub struct Behavior {
    request_receiver: mpsc::UnboundedReceiver<CircuitRequest>,
    dial_requests: HashMap<ConnectionId, CircuitRequest>,
    pending_events: VecDeque<BehaviorEvent<Infallible, THandlerAction<Self>>>,
}

impl Behavior {
    pub fn new(request_receiver: mpsc::UnboundedReceiver<CircuitRequest>) -> Self {
        Self {
            request_receiver,
            dial_requests: HashMap::new(),
            pending_events: VecDeque::new(),
        }
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = Either<DummyHandler, handler::Handler>;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        Poll::Pending
    }
}

impl NetworkOutgoingBehavior for Behavior {
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        if self.dial_requests.contains_key(&id) {
            // 如果是待处理的请求，返回对应的处理器
            Ok(Either::Right(handler::Handler::new()))
        } else {
            // 否则返回一个空的处理器
            Ok(Either::Left(DummyHandler))
        }
    }

    fn on_connection_established(&mut self, id: ConnectionId, _peer_id: PeerId, _addr: &Multiaddr) {
        // 在排队中的连接
        if let Some(request) = self.dial_requests.remove(&id) {
            // 处理拨号成功，写入连接操作
        }
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
        if let Some(request) = self.dial_requests.remove(&id) {
            // 处理拨号失败
            tracing::error!("Dial failed for request: {:?}", request.dst_peer_id);
        }
    }

    fn on_dial_failure(
        &mut self,
        id: ConnectionId,
        peer_id: Option<PeerId>,
        addr: Option<&Multiaddr>,
        error: &DialError,
    ) {
        tracing::warn!(
            "Dial failure peer id :{:?}, addr: {:?} : {:?}",
            peer_id,
            addr,
            error
        );

        if let Some(request) = self.dial_requests.remove(&id) {
            // 处理拨号失败
            tracing::error!("Dial failed for request: {:?}", request.dst_peer_id);
        }
    }

    fn poll_dial(&mut self, cx: &mut Context<'_>) -> Poll<DialOpts> {
        let Some(request) = ready!(self.request_receiver.poll_next_unpin(cx)) else {
            return Poll::Pending;
        };
        let dial_opts = DialOpts::new(None, Some(request.dst_peer_id));
        tracing::error!("Relay Dialing ....{:?}", dial_opts);

        // 关联 dial connect_id 和 Request;
        self.dial_requests
            .insert(dial_opts.connection_id(), request);
        Poll::Ready(dial_opts)
    }
}
