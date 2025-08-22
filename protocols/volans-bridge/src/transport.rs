use std::{
    collections::VecDeque,
    io,
    pin::Pin,
    task::{Context, Poll, Waker},
};

use futures::{
    AsyncRead, AsyncWrite, FutureExt, SinkExt, StreamExt,
    channel::{mpsc, oneshot},
    future::{self, BoxFuture},
    ready,
};
use volans_codec::Bytes;
use volans_core::{
    Listener, ListenerEvent, Multiaddr, PeerId, Transport, TransportError, multiaddr::Protocol,
};
use volans_swarm::Substream;

use crate::{
    MultiaddrExt,
    protocol::{Circuit, ConnectError},
};

pub struct Config {
    behavior_sender: mpsc::Sender<TransportRequest>,
}

impl Config {
    pub fn new() -> (Self, mpsc::Receiver<TransportRequest>) {
        let (behavior_sender, behavior_receiver) = mpsc::channel(1000);
        (Self { behavior_sender }, behavior_receiver)
    }
}

pub(crate) struct DialRequest {
    relay_addr: Multiaddr,
    relay_peer_id: PeerId,
    dst_addr: Option<Multiaddr>,
    dst_peer_id: PeerId,
    send_back: oneshot::Sender<Result<Connection, io::Error>>,
}

impl Transport for Config {
    type Output = Connection;
    type Error = Error;
    type Dial = future::BoxFuture<'static, Result<Self::Output, Self::Error>>;
    type Incoming = future::Ready<Result<Self::Output, Self::Error>>;
    type Listener = ListenerBackend;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        // 解析地址，获取中继地址和目标地址, 地址类型
        // /ip4/127.0.0.1/udp/10088/quic-v1/peer/{relay-server-peer}/circuit/peer/{backend-peer}
        let RelayedMultiaddr {
            relay_peer_id,
            relay_addr,
            dst_peer_id,
        } = parse_relayed_multiaddr(addr)?;
        let relay_peer_id = relay_peer_id.ok_or(Error::MissingRelayPeerId)?;
        let dst_peer_id = dst_peer_id.ok_or(Error::MissingDstPeerId)?;
        let relay_addr = relay_addr.ok_or(Error::InvalidMultiaddr)?;

        let mut behavior_sender = self.behavior_sender.clone();

        Ok(async move {
            let (tx, rx) = oneshot::channel();
            let request = TransportRequest::DialRequest {
                relay_addr,
                relay_peer_id,
                dst_peer_id,
                send_back: tx,
            };
            behavior_sender.send(request).await?;
            let stream = rx.await??;
            tracing::info!("Dialed relay peer: {}", dst_peer_id);
            Ok(stream)
        }
        .boxed())
    }
    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        if !addr.is_circuit() {
            return Err(TransportError::NotSupported(addr));
        }

        let (listener_sender, incoming_stream) = mpsc::channel(100);

        let listen_request = TransportRequest::ListenRequest {
            local_addr: addr.clone(),
            listener_sender,
        };
        tracing::trace!("new circuit listener addr: {}", addr);

        let listener = ListenerBackend {
            local_addr: addr,
            pending_request: Some(listen_request),
            behavior_sender: self.behavior_sender.clone(),
            incoming_stream,
            closed: false,
            waker: None,
            pending_events: VecDeque::new(),
        };
        Ok(listener)
    }
}

pub struct ListenerBackend {
    local_addr: Multiaddr,
    pending_request: Option<TransportRequest>,
    behavior_sender: mpsc::Sender<TransportRequest>,
    incoming_stream: mpsc::Receiver<IncomingRelayedConnection>,
    closed: bool,
    waker: Option<Waker>,
    pending_events: VecDeque<ListenerEvent<<Self as Listener>::Upgrade, <Self as Listener>::Error>>,
}

impl Listener for ListenerBackend {
    type Output = Connection;
    type Error = Error;
    type Upgrade = future::Ready<Result<Self::Output, Self::Error>>;

    fn poll_event(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(event);
            }
            if self.closed {
                self.waker = None;
                return Poll::Ready(ListenerEvent::Closed(Ok(())));
            }

            if self.pending_request.is_some() {
                if self.behavior_sender.poll_ready(cx).is_ready() {
                    if let Some(request) = self.pending_request.take() {
                        let _ = self.behavior_sender.start_send(request);
                        let addr = self.local_addr.clone();
                        self.pending_events
                            .push_back(ListenerEvent::NewAddress(addr));
                        continue;
                    }
                }
            }

            match self.incoming_stream.poll_next_unpin(cx) {
                Poll::Ready(Some(IncomingRelayedConnection {
                    stream,
                    src_peer_id,
                    relay_peer_id: _,
                    relay_addr,
                })) => {
                    tracing::info!("Received incoming relayed connection from: {}", src_peer_id);
                    self.pending_events.push_back(ListenerEvent::Incoming {
                        local_addr: relay_addr.with(Protocol::Circuit),
                        remote_addr: Protocol::Peer(src_peer_id).into(),
                        upgrade: future::ready(Ok(stream)),
                    });
                    continue;
                }
                Poll::Ready(None) => {
                    self.pending_events.push_back(ListenerEvent::Closed(Ok(())));
                    self.closed = true;
                    continue;
                }
                Poll::Pending => {
                    self.waker = Some(cx.waker().clone());
                }
            };
            return Poll::Pending;
        }
    }

    fn poll_close(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.pending_events.push_back(ListenerEvent::Closed(Ok(())));
        self.closed = true;
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
        Poll::Ready(Ok(()))
    }
}

pub struct IncomingRelayedConnection {
    stream: Connection,
    /// 源端的 PeerId
    src_peer_id: PeerId,
    /// 中继端的 PeerId
    relay_peer_id: PeerId,
    /// 中继端的地址
    relay_addr: Multiaddr,
}

impl IncomingRelayedConnection {
    pub fn new(
        stream: Connection,
        src_peer_id: PeerId,
        relay_peer_id: PeerId,
        relay_addr: Multiaddr,
    ) -> Self {
        Self {
            stream,
            src_peer_id,
            relay_peer_id,
            relay_addr,
        }
    }
}

pub enum TransportRequest {
    DialRequest {
        relay_addr: Multiaddr,
        relay_peer_id: PeerId,
        dst_peer_id: PeerId,
        send_back: oneshot::Sender<Result<Connection, ConnectError>>,
    },
    ListenRequest {
        local_addr: Multiaddr,
        listener_sender: mpsc::Sender<IncomingRelayedConnection>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Missing relay peer id")]
    MissingRelayPeerId,
    #[error("Missing destination peer id")]
    MissingDstPeerId,
    #[error("Multiple circuit addresses found")]
    MultipleCircuit,
    #[error("Invalid circuit multiaddr format")]
    InvalidMultiaddr,
    #[error("Transport not supported for address: {0}")]
    BehaviorSend(#[from] mpsc::SendError),
    #[error("Transport error: {0}")]
    BehaviorResponse(#[from] oneshot::Canceled),
    #[error("Transport error: {0}")]
    ConnectError(#[from] ConnectError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Default)]
struct RelayedMultiaddr {
    relay_peer_id: Option<PeerId>,
    relay_addr: Option<Multiaddr>,
    dst_peer_id: Option<PeerId>,
}

fn parse_relayed_multiaddr(addr: Multiaddr) -> Result<RelayedMultiaddr, TransportError<Error>> {
    if !addr.is_circuit() {
        return Err(TransportError::NotSupported(addr));
    }

    let mut relayed_multiaddr = RelayedMultiaddr::default();
    let mut before_circuit = true;
    for protocol in addr.into_iter() {
        match protocol {
            Protocol::Circuit => {
                if before_circuit {
                    before_circuit = false;
                } else {
                    return Err(Error::MultipleCircuit.into());
                }
            }
            Protocol::Peer(peer_id) if before_circuit => {
                if relayed_multiaddr.relay_peer_id.is_some() {
                    return Err(Error::InvalidMultiaddr.into());
                }
                relayed_multiaddr.relay_peer_id = Some(peer_id);
            }
            Protocol::Peer(peer_id) if !before_circuit => {
                if relayed_multiaddr.dst_peer_id.is_some() {
                    return Err(Error::InvalidMultiaddr.into());
                }
                relayed_multiaddr.dst_peer_id = Some(peer_id);
            }
            p => {
                if before_circuit {
                    relayed_multiaddr
                        .relay_addr
                        .get_or_insert(Multiaddr::empty())
                        .push(p);
                } else {
                    return Err(Error::InvalidMultiaddr.into());
                }
            }
        }
    }
    Ok(relayed_multiaddr)
}

pub struct Connection {
    pub(crate) state: ConnectionState,
}

impl Connection {
    pub(crate) fn new_accepting(circuit: Circuit) -> Self {
        Connection {
            state: ConnectionState::Accepting {
                accept: async {
                    let (substream, read_buffer) = circuit.accept().await?;
                    Ok(ConnectionState::Accepted {
                        read_buffer,
                        substream,
                    })
                }
                .boxed(),
            },
        }
    }

    pub(crate) fn new_accepted(substream: Substream, read_buffer: Bytes) -> Self {
        Connection {
            state: ConnectionState::Accepted {
                read_buffer,
                substream,
            },
        }
    }
}

pub(crate) enum ConnectionState {
    Accepting {
        accept: BoxFuture<'static, Result<ConnectionState, io::Error>>,
    },
    Accepted {
        read_buffer: Bytes,
        substream: Substream,
    },
}

impl Unpin for ConnectionState {}

impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                ConnectionState::Accepting { accept } => {
                    *self = Connection {
                        state: ready!(accept.poll_unpin(cx))?,
                    };
                }
                ConnectionState::Accepted { substream, .. } => {
                    return Pin::new(substream).poll_write(cx, buf);
                }
            }
        }
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                ConnectionState::Accepting { accept } => {
                    *self = Connection {
                        state: ready!(accept.poll_unpin(cx))?,
                    };
                }
                ConnectionState::Accepted { substream, .. } => {
                    return Pin::new(substream).poll_write_vectored(cx, bufs);
                }
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                ConnectionState::Accepting { accept } => {
                    *self = Connection {
                        state: ready!(accept.poll_unpin(cx))?,
                    };
                }
                ConnectionState::Accepted { substream, .. } => {
                    return Pin::new(substream).poll_flush(cx);
                }
            }
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        loop {
            match &mut self.state {
                ConnectionState::Accepting { accept } => {
                    *self = Connection {
                        state: ready!(accept.poll_unpin(cx))?,
                    };
                }
                ConnectionState::Accepted { substream, .. } => {
                    return Pin::new(substream).poll_close(cx);
                }
            }
        }
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match &mut self.state {
                ConnectionState::Accepting { accept } => {
                    *self = Connection {
                        state: ready!(accept.poll_unpin(cx))?,
                    };
                }
                ConnectionState::Accepted {
                    read_buffer,
                    substream,
                } => {
                    // 先从 read_buffer 中读取数据
                    if !read_buffer.is_empty() {
                        let n = read_buffer.len().min(buf.len());
                        let data = read_buffer.split_to(n);
                        buf[0..n].copy_from_slice(&data[..]);
                        return Poll::Ready(Ok(n));
                    }
                    return Pin::new(substream).poll_read(cx, buf);
                }
            }
        }
    }
}
