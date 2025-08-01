use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Stream, StreamExt, channel::oneshot, stream::SelectAll};
use smallvec::SmallVec;
use volans_core::{
    ConnectedPoint, PeerId, Transport, TransportError, Url, muxing::StreamMuxerBox, transport,
};

use crate::{
    BehaviorEvent, ConnectionId, InboundStreamHandler, ListenOpts, ListenerEvent, ListenerId,
    NetworkIncomingBehavior, PendingNotifyHandler, THandlerAction, THandlerEvent,
    behavior::{
        CloseConnection, ExpiredListenAddr, ListenerClosed, ListenerError, NewListenAddr,
        NewListener, NotifyHandler,
    },
    connection::{Pool, PoolConfig, PoolEvent},
    error::{ConnectionError, ListenError},
    listener, notify_all, notify_any, notify_one,
};

pub struct Swarm<TBehavior>
where
    TBehavior: NetworkIncomingBehavior,
{
    behavior: TBehavior,
    transport: transport::Boxed<(PeerId, StreamMuxerBox)>,
    pool: Pool<TBehavior::ConnectionHandler>,
    /// 等待Handler操作
    pending_handler_action: Option<(PeerId, PendingNotifyHandler, THandlerAction<TBehavior>)>,

    /// listeners
    listeners: SelectAll<listener::TaggedListener>,
    listeners_abort: HashMap<ListenerId, oneshot::Sender<Infallible>>,
    listened_addresses: HashMap<ListenerId, SmallVec<[Url; 1]>>,

    /// Swarm 等待处理的事件
    pending_swarm_events: VecDeque<SwarmEvent<TBehavior::Event>>,
}

impl<TBehavior> Unpin for Swarm<TBehavior> where TBehavior: NetworkIncomingBehavior {}

impl<TBehavior> Swarm<TBehavior>
where
    TBehavior: NetworkIncomingBehavior,
    TBehavior::ConnectionHandler: InboundStreamHandler,
{
    pub fn new(
        transport: transport::Boxed<(PeerId, StreamMuxerBox)>,
        behavior: TBehavior,
        local_peer_id: PeerId,
        config: PoolConfig,
    ) -> Self {
        Self {
            behavior,
            transport,
            pool: Pool::new(local_peer_id, config),
            pending_handler_action: None,
            listeners: SelectAll::new(),
            listeners_abort: HashMap::new(),
            listened_addresses: HashMap::new(),
            pending_swarm_events: VecDeque::new(),
        }
    }

    /// 关闭指定的连接
    pub fn close_connection(&mut self, connection_id: ConnectionId) -> bool {
        if let Some(established) = self.pool.get_established(connection_id) {
            established.start_close();
            return true;
        }
        false
    }

    /// 检查指定的 PeerId 是否已连接
    pub fn is_peer_connected(&self, peer_id: &PeerId) -> bool {
        self.pool.is_peer_connected(peer_id)
    }

    /// 获取所有已连接的 PeerId
    pub fn connected_peers(&self) -> impl Iterator<Item = &PeerId> {
        self.pool.iter_peer_connected()
    }

    /// 获取所有已连接的连接 ID
    pub fn connected_connections(&self) -> impl Iterator<Item = &ConnectionId> {
        self.pool.iter_connected()
    }

    pub fn behavior(&self) -> &TBehavior {
        &self.behavior
    }

    pub fn behavior_mut(&mut self) -> &mut TBehavior {
        &mut self.behavior
    }

    /// 开始监听指定的地址
    pub fn listen_on(&mut self, addr: Url) -> Result<ListenerId, TransportError<io::Error>> {
        let opts = ListenOpts::new(addr);
        let listener_id = opts.listener_id();
        let addr = opts.addr();
        match self.transport.listen(&addr) {
            Ok(listener) => {
                let (close_tx, close_rx) = oneshot::channel();
                let tagged_listener =
                    listener::TaggedListener::new(listener_id, listener, close_rx);
                self.listeners.push(tagged_listener);
                self.listeners_abort.insert(listener_id, close_tx);
            }
            Err(error) => {
                self.behavior
                    .on_listener_event(ListenerEvent::ListenerError(ListenerError {
                        listener_id,
                        error: &error,
                    }));
                return Err(error);
            }
        }
        self.behavior
            .on_listener_event(ListenerEvent::NewListener(NewListener { listener_id }));
        Ok(listener_id)
    }

    /// 获取所有监听的地址
    pub fn listeners(&self) -> impl Iterator<Item = &Url> {
        self.listened_addresses.values().flatten()
    }

    /// 移除指定的监听器
    pub fn remove_listener(&mut self, listener_id: ListenerId) -> bool {
        match self.listeners_abort.remove(&listener_id) {
            Some(abort_sender) => {
                // Drop 掉 close_sender 以触发监听器关闭
                drop(abort_sender);
                true
            }
            None => false,
        }
    }

    fn handle_behavior_event(
        &mut self,
        event: BehaviorEvent<TBehavior::Event, THandlerAction<TBehavior>>,
    ) {
        match event {
            BehaviorEvent::Behavior(event) => {
                self.pending_swarm_events
                    .push_back(SwarmEvent::Behavior(event));
            }
            BehaviorEvent::HandlerAction {
                peer_id,
                handler,
                action,
            } => {
                assert!(
                    self.pending_handler_action.is_none(),
                    "Pending handler action already exists"
                );
                let handler = match handler {
                    NotifyHandler::One(connection) => PendingNotifyHandler::One(connection),
                    NotifyHandler::Any => {
                        let ids = self
                            .pool
                            .iter_established_connections_of_peer(&peer_id)
                            .collect();
                        PendingNotifyHandler::Any(ids)
                    }
                    NotifyHandler::All => {
                        let ids = self
                            .pool
                            .iter_established_connections_of_peer(&peer_id)
                            .collect();
                        PendingNotifyHandler::All(ids)
                    }
                };
                self.pending_handler_action = Some((peer_id, handler, action));
            }
            BehaviorEvent::CloseConnection {
                peer_id,
                connection,
            } => match connection {
                CloseConnection::One(id) => {
                    if let Some(connection) = self.pool.get_established(id) {
                        connection.start_close();
                    } else {
                        tracing::debug!(
                            id = ?id,
                            peer_id = ?peer_id,
                            "Attempted to close non-existent connection"
                        );
                    }
                }
                CloseConnection::All => self.pool.disconnect(&peer_id),
            },
        }
    }

    fn handle_pool_event(&mut self, event: PoolEvent<THandlerEvent<TBehavior>>) {
        match event {
            PoolEvent::ConnectionEstablished {
                id,
                peer_id,
                endpoint,
                connection,
                established_in,
            } => {
                let (handler, local_addr, remote_addr) = match &endpoint {
                    ConnectedPoint::Dialer { addr: _ } => {
                        unreachable!("Dialer connections should not be handled here")
                    }
                    ConnectedPoint::Listener {
                        local_addr,
                        remote_addr,
                    } => match self.behavior.handle_established_connection(
                        id,
                        peer_id,
                        local_addr,
                        remote_addr,
                    ) {
                        Ok(handler) => (handler, local_addr, remote_addr),
                        Err(cause) => {
                            let listen_error = ListenError::Denied { cause };
                            self.behavior.on_listen_failure(
                                id,
                                Some(peer_id),
                                local_addr,
                                remote_addr,
                                &listen_error,
                            );
                            self.pending_swarm_events.push_back(
                                SwarmEvent::IncomingConnectionError {
                                    peer_id: Some(peer_id),
                                    connection_id: id,
                                    local_addr: local_addr.clone(),
                                    remote_addr: remote_addr.clone(),
                                    error: listen_error,
                                },
                            );
                            return;
                        }
                    },
                };

                let num_established = self.pool.num_peer_established(&peer_id);

                self.pool.spawn_inbound_connection(
                    id,
                    peer_id,
                    endpoint.clone(),
                    connection,
                    handler,
                );

                tracing::debug!(
                    peer=%peer_id,
                    local_addr=%local_addr,
                    remote_addr=%remote_addr,
                    total_peers=%num_established,
                    "Connection inbound established"
                );
                self.behavior
                    .on_connection_established(id, peer_id, local_addr, remote_addr);
                self.pending_swarm_events
                    .push_back(SwarmEvent::ConnectionEstablished {
                        connection_id: id,
                        peer_id,
                        local_addr: local_addr.clone(),
                        remote_addr: remote_addr.clone(),
                        established_in,
                        num_established,
                    });
            }
            PoolEvent::PendingConnectionError {
                id,
                peer_id,
                endpoint,
                error,
            } => match endpoint {
                ConnectedPoint::Dialer { addr: _ } => {
                    unreachable!("Dialer connections should not be handled here")
                }
                ConnectedPoint::Listener {
                    local_addr,
                    remote_addr,
                } => {
                    let listen_error = ListenError::from(error);
                    self.behavior.on_listen_failure(
                        id,
                        peer_id,
                        &local_addr,
                        &remote_addr,
                        &listen_error,
                    );
                    self.pending_swarm_events
                        .push_back(SwarmEvent::IncomingConnectionError {
                            peer_id,
                            connection_id: id,
                            local_addr,
                            remote_addr,
                            error: listen_error,
                        });
                }
            },
            PoolEvent::ConnectionClosed {
                id,
                peer_id,
                endpoint,
                num_remaining_established,
                error,
            } => match endpoint {
                ConnectedPoint::Dialer { addr: _ } => {
                    unreachable!("Dialer connections should not be handled here")
                }
                ConnectedPoint::Listener {
                    local_addr,
                    remote_addr,
                } => {
                    self.behavior.on_connection_closed(
                        id,
                        peer_id,
                        &local_addr,
                        &remote_addr,
                        error.as_ref(),
                    );
                    self.pending_swarm_events
                        .push_back(SwarmEvent::ConnectionClosed {
                            connection_id: id,
                            peer_id,
                            local_addr: local_addr.clone(),
                            remote_addr: remote_addr.clone(),
                            num_remaining_established,
                            error,
                        });
                }
            },
            PoolEvent::ConnectionEvent { id, peer_id, event } => {
                self.behavior
                    .on_connection_handler_event(id, peer_id, event);
            }
        }
    }

    fn handle_listener_event(
        &mut self,
        listener_id: ListenerId,
        event: listener::BoxedListenerEvent,
    ) {
        match event {
            transport::ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade,
            } => {
                let connection_id = ConnectionId::next();
                match self.behavior.handle_pending_connection(
                    connection_id,
                    &local_addr,
                    &remote_addr,
                ) {
                    Ok(()) => {}
                    Err(cause) => {
                        let listen_error = ListenError::Denied { cause };
                        self.behavior.on_listen_failure(
                            connection_id,
                            None,
                            &local_addr,
                            &remote_addr,
                            &listen_error,
                        );
                        self.pending_swarm_events
                            .push_back(SwarmEvent::IncomingConnectionError {
                                peer_id: None,
                                connection_id,
                                local_addr,
                                remote_addr,
                                error: listen_error,
                            });
                        return;
                    }
                }
                self.pool.add_incoming(
                    connection_id,
                    upgrade,
                    local_addr.clone(),
                    remote_addr.clone(),
                );

                self.pending_swarm_events
                    .push_back(SwarmEvent::IncomingConnection {
                        connection_id,
                        local_addr,
                        remote_addr,
                    })
            }
            transport::ListenerEvent::Listened(addr) => {
                tracing::debug!(listener = ?listener_id, addr = %addr, "Listener started");
                let addresses = self.listened_addresses.entry(listener_id).or_default();

                if !addresses.contains(&addr) {
                    addresses.push(addr.clone());
                }

                self.behavior
                    .on_listener_event(ListenerEvent::NewListenAddr(NewListenAddr {
                        listener_id,
                        addr: &addr,
                    }));
                self.pending_swarm_events
                    .push_back(SwarmEvent::NewListenAddr { listener_id, addr });
            }
            transport::ListenerEvent::Closed(reason) => {
                tracing::debug!(
                    listener=?listener_id,
                    ?reason,
                    "Listener closed"
                );
                // 移除监听器的地址
                let addresses = self
                    .listened_addresses
                    .remove(&listener_id)
                    .unwrap_or_default();
                for addr in addresses.iter() {
                    // 通知行为层监听器地址过期
                    self.behavior
                        .on_listener_event(ListenerEvent::ExpiredListenAddr(ExpiredListenAddr {
                            listener_id,
                            addr,
                        }));
                }
                self.behavior
                    .on_listener_event(ListenerEvent::ListenerClosed(ListenerClosed {
                        listener_id,
                        reason: reason.as_ref().copied(),
                    }));
                self.pending_swarm_events
                    .push_back(SwarmEvent::ListenerClosed {
                        listener_id,
                        reason,
                    });
            }
            transport::ListenerEvent::Error(error) => {
                tracing::debug!(listener = ?listener_id, "Listener error");
                self.behavior
                    .on_listener_event(ListenerEvent::ListenerError(ListenerError {
                        listener_id,
                        error: &error,
                    }));
                self.pending_swarm_events
                    .push_back(SwarmEvent::ListenerError { listener_id, error });
            }
        }
    }

    #[tracing::instrument(level = "debug", name = "Swarm::poll", skip(self, cx))]
    fn poll_next_event(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<SwarmEvent<TBehavior::Event>> {
        let this = &mut *self;
        loop {
            if let Some(event) = this.pending_swarm_events.pop_front() {
                return Poll::Ready(event);
            }
            match this.pending_handler_action.take() {
                Some((peer_id, handler, action)) => match handler {
                    PendingNotifyHandler::One(id) => match this.pool.get_established(id) {
                        Some(connection) => match notify_one(connection, action, cx) {
                            Some(action) => {
                                this.pending_handler_action = Some((peer_id, handler, action));
                            }
                            None => continue,
                        },
                        None => continue,
                    },
                    PendingNotifyHandler::Any(ids) => {
                        match notify_any::<TBehavior>(ids, &mut this.pool, action, cx) {
                            Some((pending, action)) => {
                                // 写回Pending的连接ID和操作
                                this.pending_handler_action =
                                    Some((peer_id, PendingNotifyHandler::Any(pending), action));
                            }
                            None => continue,
                        }
                    }
                    PendingNotifyHandler::All(ids) => {
                        match notify_all::<TBehavior>(ids, &mut this.pool, action, cx) {
                            Some((pending, action)) => {
                                // 写回Pending的连接ID和操作
                                this.pending_handler_action =
                                    Some((peer_id, PendingNotifyHandler::All(pending), action));
                            }
                            None => continue,
                        }
                    }
                },
                // 如果没有Pending的Handler操作，继续处理Swarm事件
                None => match this.behavior.poll(cx) {
                    Poll::Pending => {}
                    Poll::Ready(event) => {
                        this.handle_behavior_event(event);
                        continue;
                    }
                },
            }

            // 处理连接池中的事件
            match this.pool.poll(cx) {
                Poll::Pending => {}
                Poll::Ready(pool_event) => {
                    this.handle_pool_event(pool_event);
                    continue;
                }
            }

            // 处理监听器事件
            match this.listeners.poll_next_unpin(cx) {
                Poll::Ready(Some((id, event))) => {
                    this.handle_listener_event(id, event);
                    continue;
                }
                Poll::Ready(None) => {}
                Poll::Pending => {}
            }
            return Poll::Pending;
        }
    }
}

impl<TBehavior> Stream for Swarm<TBehavior>
where
    TBehavior: NetworkIncomingBehavior,
    TBehavior::ConnectionHandler: InboundStreamHandler,
{
    type Item = SwarmEvent<TBehavior::Event>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.poll_next_event(cx) {
            Poll::Ready(event) => Poll::Ready(Some(event)),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum SwarmEvent<TBehaviorEvent> {
    Behavior(TBehaviorEvent),

    NewListenAddr {
        listener_id: ListenerId,
        addr: Url,
    },

    ListenerClosed {
        listener_id: ListenerId,
        reason: Result<(), io::Error>,
    },

    ListenerError {
        listener_id: ListenerId,
        error: io::Error,
    },

    IncomingConnection {
        connection_id: ConnectionId,
        local_addr: Url,
        remote_addr: Url,
    },

    IncomingConnectionError {
        connection_id: ConnectionId,
        local_addr: Url,
        remote_addr: Url,
        error: ListenError,
        peer_id: Option<PeerId>,
    },

    ConnectionEstablished {
        peer_id: PeerId,
        connection_id: ConnectionId,
        local_addr: Url,
        remote_addr: Url,
        num_established: usize,
        established_in: std::time::Duration,
    },

    ConnectionClosed {
        connection_id: ConnectionId,
        peer_id: PeerId,
        local_addr: Url,
        remote_addr: Url,
        num_remaining_established: usize,
        error: Option<ConnectionError>,
    },
}
