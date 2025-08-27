mod task;

use std::{
    collections::HashMap,
    convert::Infallible,
    io,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use fnv::{FnvHashMap, FnvHashSet};
use futures::{
    FutureExt, StreamExt,
    channel::{mpsc, oneshot},
    stream::{FuturesUnordered, SelectAll},
};
use tracing::Instrument;
use volans_core::{
    ConnectedPoint, Multiaddr, PeerId,
    muxing::{StreamMuxerBox, StreamMuxerExt},
};

use crate::{
    ConnectionHandler, ConnectionId, ExecSwitch, Executor, InboundStreamHandler,
    OutboundStreamHandler,
    connection::{InboundConnection, OutboundConnection},
    error::{ConnectionError, PendingConnectionError},
};

/// 连接池
/// 管理连接的建立、维护和事件处理
///
/// 状态机
/// add_incoming -> pending -> Event::ConnectionEstablished -> spawn_connection -> established
/// add_outgoing -> pending -> Event::ConnectionEstablished -> spawn_connection -> established

pub struct Pool<THandler>
where
    THandler: ConnectionHandler,
{
    local_id: PeerId,

    /// 等待中的连接
    pending: HashMap<ConnectionId, PendingConnection>,
    pending_peer_connections: FnvHashMap<PeerId, FnvHashSet<ConnectionId>>,

    established: FnvHashMap<ConnectionId, EstablishedConnection<THandler::Action>>,

    /// 已建立的连接
    established_peer_connections: FnvHashMap<PeerId, FnvHashSet<ConnectionId>>,

    executor: ExecSwitch,

    /// 等待中的连接事件 Sender
    pending_connection_events_tx: mpsc::Sender<task::PendingConnectionEvent>,

    /// 等待中的连接事件 Receiver
    pending_connection_events_rx: mpsc::Receiver<task::PendingConnectionEvent>,

    /// 没有建立连接的唤醒器
    no_established_connections_waker: Option<Waker>,

    established_connection_events:
        SelectAll<mpsc::Receiver<task::EstablishedConnectionEvent<THandler::Event>>>,

    /// 新连接丢弃监听器
    new_connection_dropped_listeners: FuturesUnordered<oneshot::Receiver<StreamMuxerBox>>,

    /// 任务命令缓冲区大小
    task_command_buffer_size: usize,
    /// 最大协商入站流数量
    max_negotiating_inbound_streams: usize,
    /// 每个连接事件缓冲区大小
    per_connection_event_buffer_size: usize,
    /// 连接空闲超时
    idle_connection_timeout: Duration,
}

impl<THandler> Pool<THandler>
where
    THandler: ConnectionHandler,
{
    pub fn new(local_id: PeerId, config: PoolConfig) -> Self {
        let (pending_connection_events_tx, pending_connection_events_rx) = mpsc::channel(0);

        Pool {
            local_id,
            pending: HashMap::new(),
            pending_peer_connections: FnvHashMap::default(),
            established: FnvHashMap::default(),
            established_peer_connections: FnvHashMap::default(),
            executor: ExecSwitch::new(config.executor),
            pending_connection_events_tx,
            pending_connection_events_rx,
            no_established_connections_waker: None,
            established_connection_events: SelectAll::new(),
            new_connection_dropped_listeners: FuturesUnordered::new(),
            task_command_buffer_size: config.task_command_buffer_size,
            max_negotiating_inbound_streams: config.max_negotiating_inbound_streams,
            per_connection_event_buffer_size: config.per_connection_event_buffer_size,
            idle_connection_timeout: config.idle_connection_timeout,
        }
    }

    pub fn disconnect(&mut self, id: &PeerId) {
        //处理 Pending 的连接：1、Remove Pending Map；2、中断连接任务
        for connection in self
            .pending_peer_connections
            .remove(id)
            .into_iter()
            .flatten()
        {
            if let Some(mut pending) = self.pending.remove(&connection) {
                pending.abort();
            }
        }
        //处理已建立的连接: 给所有连接发送关闭命令
        if let Some(connections) = self.established_peer_connections.get(id) {
            for connection in connections.iter() {
                if let Some(established) = self.established.get_mut(&connection) {
                    established.start_close();
                }
            }
        }
    }

    pub(crate) fn get_established(
        &mut self,
        id: ConnectionId,
    ) -> Option<&mut EstablishedConnection<THandler::Action>> {
        self.established.get_mut(&id)
    }

    pub(crate) fn is_peer_connected(&self, id: &PeerId) -> bool {
        self.established_peer_connections.contains_key(id)
    }

    pub(crate) fn is_peer_dialing(&self, id: &PeerId) -> bool {
        if let Some(connections) = self.pending_peer_connections.get(id) {
            for connection in connections.iter() {
                if let Some(pending) = self.pending.get(connection) {
                    if matches!(pending.endpoint, ConnectedPoint::Dialer { .. }) {
                        return true;
                    }
                }
            }
        }
        return false;
    }

    pub fn iter_established_connections_of_peer(
        &mut self,
        peer_id: &PeerId,
    ) -> impl Iterator<Item = ConnectionId> + '_ {
        match self.established_peer_connections.get(peer_id) {
            Some(conns) => either::Either::Left(conns.iter().copied()),
            None => either::Either::Right(std::iter::empty()),
        }
    }

    pub fn num_peer_established(&self, peer_id: &PeerId) -> usize {
        self.established_peer_connections
            .get(peer_id)
            .map_or(0, |conns| conns.len())
    }

    pub(crate) fn iter_peer_connected(&self) -> impl Iterator<Item = &PeerId> {
        self.established_peer_connections.keys()
    }

    pub(crate) fn iter_connected(&self) -> impl Iterator<Item = &ConnectionId> {
        self.established.keys()
    }

    pub fn add_outgoing<TFut>(
        &mut self,
        id: ConnectionId,
        future: TFut,
        addr: Multiaddr,
        peer_id: Option<PeerId>,
    ) where
        TFut: Future<Output = Result<(PeerId, StreamMuxerBox), io::Error>> + Send + 'static,
    {
        let (abort_notifier, abort_receiver) = oneshot::channel();
        let span = tracing::debug_span!(parent: tracing::Span::none(), "new_outgoing_connection", id = %id, peer_id = ?peer_id, remote_addr = %addr);
        span.follows_from(tracing::Span::current());
        self.executor.spawn(
            task::new_for_pending_connection(
                id,
                addr.clone(),
                future,
                abort_receiver,
                self.pending_connection_events_tx.clone(),
            )
            .instrument(span),
        );
        if let Some(peer_id) = peer_id {
            self.pending_peer_connections
                .entry(peer_id)
                .or_default()
                .insert(id);
        }
        self.pending.insert(
            id,
            PendingConnection {
                peer_id,
                endpoint: ConnectedPoint::Dialer { addr },
                abort_notifier: Some(abort_notifier),
                accepted_at: Instant::now(),
            },
        );
    }

    pub fn add_incoming<TFut>(
        &mut self,
        id: ConnectionId,
        future: TFut,
        local_addr: Multiaddr,
        remote_addr: Multiaddr,
    ) where
        TFut: Future<Output = Result<(PeerId, StreamMuxerBox), io::Error>> + Send + 'static,
    {
        let (abort_notifier, abort_receiver) = oneshot::channel();
        let span = tracing::debug_span!(parent: tracing::Span::none(), "new_incoming_connection", id = %id, local_addr = %local_addr, remote_addr = %remote_addr);
        span.follows_from(tracing::Span::current());
        self.executor.spawn(
            task::new_for_pending_connection(
                id,
                remote_addr.clone(),
                future,
                abort_receiver,
                self.pending_connection_events_tx.clone(),
            )
            .instrument(span),
        );
        self.pending.insert(
            id,
            PendingConnection {
                peer_id: None,
                endpoint: ConnectedPoint::Listener {
                    local_addr,
                    remote_addr,
                },
                abort_notifier: Some(abort_notifier),
                accepted_at: Instant::now(),
            },
        );
    }

    pub fn spawn_inbound_connection(
        &mut self,
        id: ConnectionId,
        obtained_peer_id: PeerId,
        endpoint: ConnectedPoint,
        connection: NewConnection,
        handler: THandler,
    ) where
        THandler: InboundStreamHandler,
    {
        let muxer = connection.extract();
        let established_peer_connections = self
            .established_peer_connections
            .entry(obtained_peer_id)
            .or_default();

        let (command_tx, command_rx) = mpsc::channel(self.task_command_buffer_size);
        let (event_tx, event_rx) = mpsc::channel(self.per_connection_event_buffer_size);
        // 创建连接处理器
        self.established.insert(
            id,
            EstablishedConnection {
                endpoint,
                sender: command_tx,
            },
        );
        // 将连接 ID 添加到已建立的连接列表
        established_peer_connections.insert(id);
        self.established_connection_events.push(event_rx);
        if let Some(waker) = Option::take(&mut self.no_established_connections_waker) {
            waker.wake();
        }
        let span = tracing::debug_span!(parent: tracing::Span::none(), "new_inbound_established", %id, peer = %obtained_peer_id);
        span.follows_from(tracing::Span::current());
        let connection = InboundConnection::new(
            muxer,
            handler,
            self.max_negotiating_inbound_streams,
            self.idle_connection_timeout,
        );
        self.executor.spawn(
            task::new_for_established_connection(
                id,
                obtained_peer_id,
                connection,
                command_rx,
                event_tx,
            )
            .instrument(span),
        );
    }

    pub fn spawn_outbound_connection(
        &mut self,
        id: ConnectionId,
        obtained_peer_id: PeerId,
        endpoint: ConnectedPoint,
        connection: NewConnection,
        handler: THandler,
    ) where
        THandler: OutboundStreamHandler,
    {
        let muxer = connection.extract();
        let established_peer_connections = self
            .established_peer_connections
            .entry(obtained_peer_id)
            .or_default();

        let (command_tx, command_rx) = mpsc::channel(self.task_command_buffer_size);
        let (event_tx, event_rx) = mpsc::channel(self.per_connection_event_buffer_size);
        // 创建连接处理器
        self.established.insert(
            id,
            EstablishedConnection {
                endpoint,
                sender: command_tx,
            },
        );
        // 将连接 ID 添加到已建立的连接列表
        established_peer_connections.insert(id);
        self.established_connection_events.push(event_rx);
        if let Some(waker) = Option::take(&mut self.no_established_connections_waker) {
            waker.wake();
        }
        let span = tracing::debug_span!(parent: tracing::Span::none(), "new_outbound_established", %id, peer = %obtained_peer_id);
        span.follows_from(tracing::Span::current());
        let connection = OutboundConnection::new(muxer, handler, self.idle_connection_timeout);
        self.executor.spawn(
            task::new_for_established_connection(
                id,
                obtained_peer_id,
                connection,
                command_rx,
                event_tx,
            )
            .instrument(span),
        );
    }

    #[tracing::instrument(level = "debug", name = "Pool::poll", skip(self, cx))]
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<PoolEvent<THandler::Event>> {
        match self.established_connection_events.poll_next_unpin(cx) {
            Poll::Pending => {}
            Poll::Ready(None) => {
                // 如果没有更多的连接事件，设置唤醒器
                self.no_established_connections_waker = Some(cx.waker().clone());
            }
            Poll::Ready(Some(task::EstablishedConnectionEvent::Notify { id, peer_id, event })) => {
                return Poll::Ready(PoolEvent::ConnectionEvent { id, peer_id, event });
            }
            Poll::Ready(Some(task::EstablishedConnectionEvent::Closed { id, peer_id, error })) => {
                if let Some(connections) = self.established_peer_connections.get_mut(&peer_id) {
                    connections.remove(&id);
                    if connections.is_empty() {
                        self.established_peer_connections.remove(&peer_id);
                    }
                }
                let EstablishedConnection { endpoint, .. } = self
                    .established
                    .remove(&id)
                    .expect("Connection should be established before being closed");

                let num_remaining_established = self
                    .established_peer_connections
                    .get(&peer_id)
                    .map_or(0, |conns| conns.len());

                return Poll::Ready(PoolEvent::ConnectionClosed {
                    id,
                    peer_id,
                    endpoint,
                    num_remaining_established,
                    error,
                });
            }
        }
        loop {
            if let Poll::Ready(Some(result)) =
                self.new_connection_dropped_listeners.poll_next_unpin(cx)
            {
                if let Ok(dropped_connection) = result {
                    self.executor.spawn(async move {
                        let _ = dropped_connection.close().await;
                    });
                }
                continue;
            }

            let event = match self.pending_connection_events_rx.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => event,
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => unreachable!("Pool holds both sender and receiver."),
            };

            let id = event.id();
            let PendingConnection {
                peer_id: expected_peer_id,
                endpoint,
                abort_notifier: _,
                accepted_at,
            } = self
                .pending
                .remove(&id)
                .expect("Pending connection should exist before being established");

            match event {
                // 处理连接建立事件
                task::PendingConnectionEvent::ConnectionEstablished {
                    id,
                    peer_id: obtained_peer_id,
                    muxer,
                } => {
                    // 检查是否有预期的 PeerId
                    if let Some(peer_id) = expected_peer_id {
                        if peer_id != peer_id {
                            let err_event = match &endpoint {
                                ConnectedPoint::Dialer { .. } => {
                                    PoolEvent::PendingConnectionError {
                                        id,
                                        peer_id: Some(peer_id),
                                        endpoint,
                                        error: PendingConnectionError::WrongPeerId {
                                            obtained: peer_id,
                                        },
                                    }
                                }
                                ConnectedPoint::Listener { .. } => unreachable!(
                                    "Listener connections should not have peer ID mismatch"
                                ),
                            };
                            return Poll::Ready(err_event);
                        }
                    }
                    // 是否是本地回环
                    if self.local_id == obtained_peer_id {
                        let err_event = match &endpoint {
                            ConnectedPoint::Dialer { .. } => PoolEvent::PendingConnectionError {
                                id,
                                peer_id: expected_peer_id,
                                endpoint,
                                error: PendingConnectionError::LocalPeerId,
                            },
                            ConnectedPoint::Listener { .. } => PoolEvent::PendingConnectionError {
                                id,
                                peer_id: expected_peer_id,
                                endpoint,
                                error: PendingConnectionError::LocalPeerId,
                            },
                        };
                        return Poll::Ready(err_event);
                    }
                    let established_in = accepted_at.elapsed();

                    let (connection, drop_listener) = NewConnection::new(muxer);
                    self.new_connection_dropped_listeners.push(drop_listener);

                    return Poll::Ready(PoolEvent::ConnectionEstablished {
                        id,
                        peer_id: obtained_peer_id,
                        endpoint,
                        connection,
                        established_in,
                    });
                }
                // 处理入站连接错误
                task::PendingConnectionEvent::PendingFailed { id, error } => {
                    return Poll::Ready(PoolEvent::PendingConnectionError {
                        id,
                        peer_id: expected_peer_id,
                        endpoint,
                        error,
                    });
                }
            }
        }
    }
}

pub(crate) struct PendingConnection {
    peer_id: Option<PeerId>,
    endpoint: ConnectedPoint,
    abort_notifier: Option<oneshot::Sender<Infallible>>,
    accepted_at: Instant,
}

impl PendingConnection {
    fn abort(&mut self) {
        if let Some(notifier) = self.abort_notifier.take() {
            drop(notifier);
        }
    }
}

#[derive(Debug)]
pub struct EstablishedConnection<TAction> {
    endpoint: ConnectedPoint,
    sender: mpsc::Sender<task::Command<TAction>>,
}

impl<TAction> EstablishedConnection<TAction> {
    pub(crate) fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), ()>> {
        self.sender.poll_ready(cx).map_err(|_| ())
    }

    pub(crate) fn start_send(&mut self, action: TAction) -> Result<(), ()> {
        self.sender
            .start_send(task::Command::Action(action))
            .map_err(|_| ())
    }

    pub(crate) fn start_close(&mut self) {
        match self.sender.clone().try_send(task::Command::Close) {
            Ok(()) => {}
            Err(e) => assert!(e.is_disconnected(), "No capacity for close command."),
        };
    }
}

#[derive(Debug)]
pub enum PoolEvent<TEvent> {
    ConnectionEstablished {
        id: ConnectionId,
        peer_id: PeerId,
        endpoint: ConnectedPoint,
        connection: NewConnection,
        established_in: Duration,
    },

    PendingConnectionError {
        id: ConnectionId,
        peer_id: Option<PeerId>,
        endpoint: ConnectedPoint,
        error: PendingConnectionError,
    },

    ConnectionClosed {
        id: ConnectionId,
        peer_id: PeerId,
        endpoint: ConnectedPoint,
        num_remaining_established: usize,
        error: Option<ConnectionError>,
    },
    ConnectionEvent {
        id: ConnectionId,
        peer_id: PeerId,
        event: TEvent,
    },
}

#[derive(Debug)]
pub struct NewConnection {
    connection: Option<StreamMuxerBox>,
    drop_sender: Option<oneshot::Sender<StreamMuxerBox>>,
}

impl NewConnection {
    fn new(conn: StreamMuxerBox) -> (Self, oneshot::Receiver<StreamMuxerBox>) {
        let (sender, receiver) = oneshot::channel();

        (
            Self {
                connection: Some(conn),
                drop_sender: Some(sender),
            },
            receiver,
        )
    }

    fn extract(mut self) -> StreamMuxerBox {
        self.connection
            .take()
            .expect("Connection should be available when extracted")
    }
}

impl Drop for NewConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            let _ = self
                .drop_sender
                .take()
                .expect("`drop_sender` to always be `Some`")
                .send(connection);
        }
    }
}

pub struct PoolConfig {
    executor: Box<dyn Executor + Send>,
    task_command_buffer_size: usize,
    per_connection_event_buffer_size: usize,
    idle_connection_timeout: Duration,
    max_negotiating_inbound_streams: usize,
}

impl PoolConfig {
    pub fn new(executor: Box<dyn Executor + Send>) -> Self {
        Self {
            executor,
            task_command_buffer_size: 32,
            per_connection_event_buffer_size: 10,
            idle_connection_timeout: Duration::from_secs(60),
            max_negotiating_inbound_streams: 128,
        }
    }

    pub fn with_task_command_buffer_size(mut self, size: usize) -> Self {
        self.task_command_buffer_size = size;
        self
    }

    pub fn with_per_connection_event_buffer_size(mut self, size: usize) -> Self {
        self.per_connection_event_buffer_size = size;
        self
    }

    pub fn with_idle_connection_timeout(mut self, timeout: Duration) -> Self {
        self.idle_connection_timeout = timeout;
        self
    }

    pub fn with_max_negotiating_inbound_streams(mut self, count: usize) -> Self {
        self.max_negotiating_inbound_streams = count;
        self
    }
}
