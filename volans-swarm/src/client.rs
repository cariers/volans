use std::{
    collections::VecDeque,
    pin::Pin,
    task::{Context, Poll},
};

use futures::Stream;
use volans_core::{ConnectedPoint, PeerId, Transport, Url, muxing::StreamMuxerBox, transport};

use crate::{
    BehaviorEvent, ConnectionId, DialOpts, NetworkOutgoingBehavior, OutboundStreamHandler,
    PeerCondition, PendingNotifyHandler, THandlerAction, THandlerEvent,
    behavior::{CloseConnection, NotifyHandler},
    connection::{Pool, PoolConfig, PoolEvent},
    error::{ConnectionError, DialError},
    notify_all, notify_any, notify_one,
};

pub struct Swarm<TBehavior>
where
    TBehavior: NetworkOutgoingBehavior,
    TBehavior::ConnectionHandler: OutboundStreamHandler,
{
    behavior: TBehavior,
    transport: transport::Boxed<(PeerId, StreamMuxerBox)>,
    pool: Pool<TBehavior::ConnectionHandler>,
    /// 等待Handler操作
    pending_handler_action: Option<(PeerId, PendingNotifyHandler, THandlerAction<TBehavior>)>,

    /// Swarm 等待处理的事件
    pending_swarm_events: VecDeque<SwarmEvent<TBehavior::Event>>,
}

impl<TBehavior> Unpin for Swarm<TBehavior>
where
    TBehavior: NetworkOutgoingBehavior,
    TBehavior::ConnectionHandler: OutboundStreamHandler,
{
}

impl<TBehavior> Swarm<TBehavior>
where
    TBehavior: NetworkOutgoingBehavior,
    TBehavior::ConnectionHandler: OutboundStreamHandler,
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

    /// 创建一个新的 Swarm 实例
    pub fn dial(&mut self, opts: DialOpts) -> Result<(), DialError> {
        let peer_id = opts.peer_id();
        let condition = opts.condition();
        let connection_id = opts.connection_id();
        let addr = opts.addr();

        // 是否可以建立连接
        let should_dial = match (condition, peer_id) {
            (_, None) => true,
            (PeerCondition::Always, _) => true,
            (PeerCondition::Disconnected, Some(ref peer_id)) => {
                !self.pool.is_peer_connected(peer_id)
            }
            (PeerCondition::NotDialing, Some(ref peer_id)) => !self.pool.is_peer_dialing(peer_id),
            (PeerCondition::DisconnectedAndNotDialing, Some(ref peer_id)) => {
                !self.pool.is_peer_dialing(peer_id) && !self.pool.is_peer_connected(peer_id)
            }
        };
        if !should_dial {
            let err = DialError::PeerCondition(condition);
            self.behavior
                .on_dial_failure(connection_id, peer_id, Some(&addr), &err);
            return Err(err);
        }
        // 1.开始执行Transport 连接，
        let future = match self.transport.dial(&opts.addr()) {
            Ok(dial) => dial,
            Err(error) => {
                let err = DialError::Transport {
                    addr: addr.clone(),
                    error,
                };
                self.behavior
                    .on_dial_failure(connection_id, peer_id, Some(&addr), &err);
                return Err(err);
            }
        };
        // 2.加入Connection Pool
        self.pool.add_outgoing(connection_id, future, addr, peer_id);
        Ok(())
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
                let (handler, addr) = match &endpoint {
                    ConnectedPoint::Dialer { addr } => match self
                        .behavior
                        .handle_established_connection(id, peer_id, addr)
                    {
                        Ok(handler) => (handler, addr.clone()),
                        Err(cause) => {
                            let dial_error = DialError::Denied { cause };
                            self.behavior.on_dial_failure(
                                id,
                                Some(peer_id),
                                Some(addr),
                                &dial_error,
                            );
                            self.pending_swarm_events
                                .push_back(SwarmEvent::ConnectionError {
                                    peer_id: Some(peer_id),
                                    connection_id: id,
                                    addr: Some(addr.clone()),
                                    error: dial_error,
                                });
                            return;
                        }
                    },
                    ConnectedPoint::Listener { .. } => {
                        unreachable!("Listener connections should not be handled here")
                    }
                };

                let num_established = self.pool.num_peer_established(&peer_id);

                self.pool.spawn_outbound_connection(
                    id,
                    peer_id,
                    endpoint.clone(),
                    connection,
                    handler,
                );
                tracing::debug!(
                    peer=%peer_id,
                    addr=%addr,
                    total_peers=%num_established,
                    "Connection outbound established"
                );
                self.behavior.on_connection_established(id, peer_id, &addr);
                self.pending_swarm_events
                    .push_back(SwarmEvent::ConnectionEstablished {
                        connection_id: id,
                        peer_id,
                        addr,
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
                ConnectedPoint::Dialer { addr } => {
                    let dial_error = DialError::from(error);
                    self.behavior
                        .on_dial_failure(id, peer_id, Some(&addr), &dial_error);
                    self.pending_swarm_events
                        .push_back(SwarmEvent::ConnectionError {
                            peer_id,
                            connection_id: id,
                            addr: Some(addr),
                            error: dial_error,
                        });
                }
                ConnectedPoint::Listener { .. } => {
                    unreachable!("Dialer connections should not be handled here")
                }
            },
            PoolEvent::ConnectionClosed {
                id,
                peer_id,
                endpoint,
                num_remaining_established,
                error,
            } => match endpoint {
                ConnectedPoint::Dialer { addr } => {
                    self.behavior
                        .on_connection_closed(id, peer_id, &addr, error.as_ref());
                    self.pending_swarm_events
                        .push_back(SwarmEvent::ConnectionClosed {
                            connection_id: id,
                            peer_id,
                            addr,
                            num_remaining_established,
                            error,
                        });
                }
                ConnectedPoint::Listener { .. } => {
                    unreachable!("Dialer connections should not be handled here")
                }
            },
            PoolEvent::ConnectionEvent { id, peer_id, event } => {
                self.behavior
                    .on_connection_handler_event(id, peer_id, event);
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

            match this.behavior.poll_dial(cx) {
                Poll::Pending => {}
                Poll::Ready(opts) => {
                    let peer_id = opts.peer_id();
                    let connection_id = opts.connection_id();
                    let addr = opts.addr();

                    if let Ok(()) = this.dial(opts) {
                        this.pending_swarm_events.push_back(SwarmEvent::Dialing {
                            peer_id,
                            connection_id,
                            addr,
                        });
                    }
                }
            }

            // 处理连接池中的事件
            match this.pool.poll(cx) {
                Poll::Pending => {}
                Poll::Ready(pool_event) => {
                    this.handle_pool_event(pool_event);
                    continue;
                }
            }
            return Poll::Pending;
        }
    }
}

impl<TBehavior> Stream for Swarm<TBehavior>
where
    TBehavior: NetworkOutgoingBehavior,
    TBehavior::ConnectionHandler: OutboundStreamHandler,
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

    Dialing {
        peer_id: Option<PeerId>,
        addr: Url,
        connection_id: ConnectionId,
    },

    ConnectionError {
        peer_id: Option<PeerId>,
        connection_id: ConnectionId,
        addr: Option<Url>,
        error: DialError,
    },

    ConnectionEstablished {
        peer_id: PeerId,
        connection_id: ConnectionId,
        addr: Url,
        num_established: usize,
        established_in: std::time::Duration,
    },

    ConnectionClosed {
        connection_id: ConnectionId,
        peer_id: PeerId,
        addr: Url,
        num_remaining_established: usize,
        error: Option<ConnectionError>,
    },
}
