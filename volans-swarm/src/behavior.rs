mod either;
mod listen_addresses;

pub use listen_addresses::ListenAddresses;

use std::task::{Context, Poll};

use volans_core::{Multiaddr, PeerId};

use crate::{
    ConnectionDenied, ConnectionHandler, ConnectionId, DialOpts, ListenerId, THandlerAction,
    THandlerEvent,
    error::{ConnectionError, DialError, ListenError},
};

pub trait NetworkBehavior: Send + 'static {
    type Event: Send + 'static;
    type ConnectionHandler: ConnectionHandler;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    );

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>>;
}

pub trait NetworkIncomingBehavior: NetworkBehavior {
    /// 处理新的入站连接
    fn handle_pending_connection(
        &mut self,
        _id: ConnectionId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<(), ConnectionDenied> {
        Ok(())
    }

    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied>;

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

pub trait NetworkOutgoingBehavior: NetworkBehavior {
    fn handle_pending_connection(
        &mut self,
        _id: ConnectionId,
        _maybe_peer: Option<PeerId>,
        _addr: &Option<Multiaddr>,
    ) -> Result<Option<Multiaddr>, ConnectionDenied> {
        Ok(None)
    }

    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied>;

    /// 连接处理器事件处理
    fn on_connection_established(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Multiaddr,
    ) {
    }

    fn on_connection_closed(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
    }

    /// 失败事件处理
    fn on_dial_failure(
        &mut self,
        _id: ConnectionId,
        _peer_id: Option<PeerId>,
        _addr: Option<&Multiaddr>,
        _error: &DialError,
    ) {
    }

    fn poll_dial(&mut self, _cx: &mut Context<'_>) -> Poll<DialOpts> {
        Poll::Pending
    }
}

#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ListenerEvent<'a> {
    /// 新的监听器事件
    NewListener(NewListener),
    /// 新的监听地址事件
    NewListenAddr(NewListenAddr<'a>),
    /// 过期的监听地址事件
    ExpiredListenAddr(ExpiredListenAddr<'a>),
    /// 监听器错误事件
    ListenerError(ListenerError<'a>),
    /// 监听器关闭事件
    ListenerClosed(ListenerClosed<'a>),
}

#[derive(Debug, Clone, Copy)]
pub struct NewListener {
    pub listener_id: ListenerId,
}

#[derive(Debug, Clone, Copy)]
pub struct NewListenAddr<'a> {
    pub listener_id: ListenerId,
    pub addr: &'a Multiaddr,
}

#[derive(Debug, Clone, Copy)]
pub struct ListenerError<'a> {
    pub listener_id: ListenerId,
    pub error: &'a (dyn std::error::Error + 'static),
}

#[derive(Debug, Clone, Copy)]
pub struct ListenerClosed<'a> {
    pub listener_id: ListenerId,
    pub reason: Result<(), &'a std::io::Error>,
}

#[derive(Debug, Clone, Copy)]
pub struct ExpiredListenAddr<'a> {
    pub listener_id: ListenerId,
    pub addr: &'a Multiaddr,
}

#[derive(Debug)]
#[non_exhaustive]
pub enum BehaviorEvent<TEvent, THandlerAction> {
    /// 行为生成的事件
    Behavior(TEvent),
    /// 处理程序命令
    HandlerAction {
        peer_id: PeerId,
        handler: NotifyHandler,
        action: THandlerAction,
    },
    /// 关闭连接事件
    CloseConnection {
        peer_id: PeerId,
        connection: CloseConnection,
    },
}

impl<TEvent, THandlerAction> BehaviorEvent<TEvent, THandlerAction> {
    pub fn map_handler_action<O, F>(self, f: F) -> BehaviorEvent<TEvent, O>
    where
        F: FnOnce(THandlerAction) -> O,
    {
        match self {
            BehaviorEvent::Behavior(event) => BehaviorEvent::Behavior(event),
            BehaviorEvent::HandlerAction {
                peer_id,
                handler,
                action,
            } => BehaviorEvent::HandlerAction {
                peer_id,
                handler,
                action: f(action),
            },
            BehaviorEvent::CloseConnection {
                peer_id,
                connection,
            } => BehaviorEvent::CloseConnection {
                peer_id,
                connection,
            },
        }
    }

    pub fn map_event<O, F>(self, f: F) -> BehaviorEvent<O, THandlerAction>
    where
        F: FnOnce(TEvent) -> O,
    {
        match self {
            BehaviorEvent::Behavior(event) => BehaviorEvent::Behavior(f(event)),
            BehaviorEvent::HandlerAction {
                peer_id,
                handler,
                action,
            } => BehaviorEvent::HandlerAction {
                peer_id,
                handler,
                action,
            },
            BehaviorEvent::CloseConnection {
                peer_id,
                connection,
            } => BehaviorEvent::CloseConnection {
                peer_id,
                connection,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum NotifyHandler {
    One(ConnectionId),
    Any,
}

#[derive(Debug, Clone, Default)]
pub enum CloseConnection {
    One(ConnectionId),
    #[default]
    All,
}
