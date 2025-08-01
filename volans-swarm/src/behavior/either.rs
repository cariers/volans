use std::task::{Context, Poll};

use either::Either;
use volans_core::{PeerId, Url};

use crate::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, NetworkOutgoingBehavior, THandler, THandlerAction, THandlerEvent,
    error::{ConnectionError, DialError, ListenError},
};

impl<L, R> NetworkBehavior for Either<L, R>
where
    L: NetworkBehavior,
    R: NetworkBehavior,
{
    type ConnectionHandler = Either<THandler<L>, THandler<R>>;
    type Event = Either<L::Event, R::Event>;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        match (self, event) {
            (Either::Left(left), Either::Left(event)) => {
                left.on_connection_handler_event(id, peer_id, event)
            }
            (Either::Right(right), Either::Right(event)) => {
                right.on_connection_handler_event(id, peer_id, event)
            }
            _ => unreachable!(),
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        match self {
            Either::Left(left) => left
                .poll(cx)
                .map(|e| e.map_event(Either::Left).map_handler_action(Either::Left)),
            Either::Right(right) => right
                .poll(cx)
                .map(|e| e.map_event(Either::Right).map_handler_action(Either::Right)),
        }
    }
}

impl<L, R> NetworkIncomingBehavior for Either<L, R>
where
    L: NetworkIncomingBehavior,
    R: NetworkIncomingBehavior,
{
    /// 处理新的入站连接
    fn handle_pending_connection(
        &mut self,
        id: ConnectionId,
        local_addr: &Url,
        remote_addr: &Url,
    ) -> Result<(), ConnectionDenied> {
        match self {
            Either::Left(left) => left.handle_pending_connection(id, local_addr, remote_addr),
            Either::Right(right) => right.handle_pending_connection(id, local_addr, remote_addr),
        }
    }

    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Url,
        remote_addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        match self {
            Either::Left(left) => left
                .handle_established_connection(id, peer_id, local_addr, remote_addr)
                .map(Either::Left),
            Either::Right(right) => right
                .handle_established_connection(id, peer_id, local_addr, remote_addr)
                .map(Either::Right),
        }
    }

    /// 连接处理器事件处理
    fn on_connection_established(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Url,
        remote_addr: &Url,
    ) {
        match self {
            Either::Left(left) => {
                left.on_connection_established(id, peer_id, local_addr, remote_addr)
            }
            Either::Right(right) => {
                right.on_connection_established(id, peer_id, local_addr, remote_addr)
            }
        }
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Url,
        remote_addr: &Url,
        reason: Option<&ConnectionError>,
    ) {
        match self {
            Either::Left(left) => {
                left.on_connection_closed(id, peer_id, local_addr, remote_addr, reason)
            }
            Either::Right(right) => {
                right.on_connection_closed(id, peer_id, local_addr, remote_addr, reason)
            }
        }
    }

    /// 监听失败事件处理
    fn on_listen_failure(
        &mut self,
        id: ConnectionId,
        peer_id: Option<PeerId>,
        local_addr: &Url,
        remote_addr: &Url,
        error: &ListenError,
    ) {
        match self {
            Either::Left(left) => {
                left.on_listen_failure(id, peer_id, local_addr, remote_addr, error)
            }
            Either::Right(right) => {
                right.on_listen_failure(id, peer_id, local_addr, remote_addr, error)
            }
        }
    }

    /// 监听器事件处理
    fn on_listener_event(&mut self, event: ListenerEvent<'_>) {
        match self {
            Either::Left(left) => left.on_listener_event(event),
            Either::Right(right) => right.on_listener_event(event),
        }
    }
}

impl<L, R> NetworkOutgoingBehavior for Either<L, R>
where
    L: NetworkOutgoingBehavior,
    R: NetworkOutgoingBehavior,
{
    fn handle_pending_connection(
        &mut self,
        id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addr: &Option<Url>,
    ) -> Result<Option<Url>, ConnectionDenied> {
        match self {
            Either::Left(left) => left.handle_pending_connection(id, maybe_peer, addr),
            Either::Right(right) => right.handle_pending_connection(id, maybe_peer, addr),
        }
    }

    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        match self {
            Either::Left(left) => left
                .handle_established_connection(id, peer_id, addr)
                .map(Either::Left),
            Either::Right(right) => right
                .handle_established_connection(id, peer_id, addr)
                .map(Either::Right),
        }
    }

    /// 连接处理器事件处理
    fn on_connection_established(&mut self, id: ConnectionId, peer_id: PeerId, addr: &Url) {
        match self {
            Either::Left(left) => left.on_connection_established(id, peer_id, addr),
            Either::Right(right) => right.on_connection_established(id, peer_id, addr),
        }
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        addr: &Url,
        reason: Option<&ConnectionError>,
    ) {
        match self {
            Either::Left(left) => left.on_connection_closed(id, peer_id, addr, reason),
            Either::Right(right) => right.on_connection_closed(id, peer_id, addr, reason),
        }
    }

    /// 失败事件处理
    fn on_dial_failure(
        &mut self,
        id: ConnectionId,
        peer_id: Option<PeerId>,
        addr: Option<&Url>,
        error: &DialError,
    ) {
        match self {
            Either::Left(left) => left.on_dial_failure(id, peer_id, addr, error),
            Either::Right(right) => right.on_dial_failure(id, peer_id, addr, error),
        }
    }

    fn poll_dial(&mut self, cx: &mut Context<'_>) -> Poll<DialOpts> {
        match self {
            Either::Left(left) => left.poll_dial(cx),
            Either::Right(right) => right.poll_dial(cx),
        }
    }
}
