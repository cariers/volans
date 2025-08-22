use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncBufRead, AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt, StreamExt, channel::oneshot,
    future::BoxFuture, io::BufReader, ready, stream::FuturesUnordered,
};
use futures_bounded::FuturesSet;
use volans_codec::Bytes;
use volans_core::{Multiaddr, OutboundUpgrade, PeerId, UpgradeInfo, upgrade::ReadyUpgrade};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol,
};

use crate::{protocol, relay::CircuitRequest};

/// 中继服务器处理连接Backend的请求
pub struct Handler {
    requested_streams: VecDeque<CircuitRequest>,
    /// 待向Backend发送的请求
    pending_streams: VecDeque<CircuitRequest>,
    circuits: FuturesUnordered<BoxFuture<'static, Result<(), io::Error>>>,
    /// 正在和Backend建立连接的流
    outbound_circuit_requests: FuturesSet<Result<CircuitParts, protocol::ConnectError>>,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            requested_streams: VecDeque::new(),
            pending_streams: VecDeque::new(),
            circuits: FuturesUnordered::new(),
            outbound_circuit_requests: FuturesSet::new(
                || futures_bounded::Delay::futures_timer(Duration::from_secs(5)),
                10, // 最大同时处理
            ),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = CircuitRequest;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        // 待向Backend发送的请求
        tracing::info!(
            "Received outbound request to relay: {:?} -> {:?}",
            action.src_peer_id,
            action.dst_peer_id
        );
        self.requested_streams.push_back(action);
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            match self.outbound_circuit_requests.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(CircuitParts {
                    mut src_stream,
                    src_pending_data,
                    dst_peer_id: _,
                    mut dst_stream,
                    dst_pending_data,
                }))) => {
                    // 创建流之间的复制任务
                    let copy_fut = async move {
                        let (result_1, result_2) = futures::future::join(
                            src_stream.write_all(&dst_pending_data),
                            dst_stream.write_all(&src_pending_data),
                        )
                        .await;
                        result_1?;
                        result_2?;

                        let copy_fut = CopyFuture::new(src_stream, dst_stream);

                        tracing::info!("Copy ...stream");
                        copy_fut.await?;
                        Ok(())
                    };
                    self.circuits.push(copy_fut.boxed());
                    continue;
                }
                Poll::Ready(Ok(Err(e))) => {
                    tracing::error!("Outbound circuit error: {:?}", e);
                    continue;
                }
                Poll::Ready(Err(_)) => {
                    tracing::error!("Outbound circuit request failed: Timeout");
                    continue;
                }
                Poll::Pending => {}
            }

            match self.circuits.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(()))) => {
                    tracing::debug!("Circuit copy completed successfully");
                    continue;
                }
                Poll::Ready(Some(Err(e))) => {
                    tracing::error!("Circuit copy failed: {:?}", e);
                    continue;
                }

                Poll::Ready(None) | Poll::Pending => {}
            }
            return Poll::Pending;
        }
    }

    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }
}

impl OutboundStreamHandler for Handler {
    type OutboundUpgrade = ReadyUpgrade<StreamProtocol>;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        stream: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        //Backend 流建立完成。
        let CircuitRequest {
            relayed_addr,
            dst_peer_id,
            dst_addresses: _,
            circuit,
            src_peer_id,
            src_connection_id,
        } = self.pending_streams.pop_front().expect("No pending stream");
        // 将流与流之间进行绑定
        tracing::debug!(
            "Relay client established connection to backend: {:?} -> {:?} src connection: {:?}",
            src_peer_id,
            dst_peer_id,
            src_connection_id,
        );

        let connect_fut =
            protocol::make_bridge_relay_connect(stream, src_peer_id, dst_peer_id, relayed_addr);

        let fut = async move {
            let (dst_stream, dst_read_buffer) = match connect_fut.await {
                Ok(dst) => dst,
                Err(protocol::ConnectError::BridgeCode(code)) => {
                    circuit.deny(code).await?;
                    return Err(protocol::ConnectError::BridgeCode(code));
                }
                Err(e) => {
                    return Err(e);
                }
            };
            let (src_stream, src_read_buffer) = circuit.accept().await?;
            Ok(CircuitParts {
                src_stream,
                src_pending_data: src_read_buffer,
                dst_peer_id,
                dst_stream,
                dst_pending_data: dst_read_buffer,
            })
        };
        let result = self.outbound_circuit_requests.try_push(fut.boxed());
        if result.is_err() {
            tracing::error!("Failed to push outbound circuit request, Dropping stream");
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        _error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        // 升级失败，通知请求者
        let request = self.pending_streams.pop_front().expect("No pending stream");
        tracing::error!("Upgrade failed for request: {:?}", request);
    }

    fn poll_outbound_request(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if let Some(request) = self.requested_streams.pop_front() {
            // 将请求发送到待处理的流
            tracing::info!(
                "Relay client processing outbound request: {:?} -> {:?}",
                request.src_peer_id,
                request.dst_peer_id
            );
            self.pending_streams.push_back(request);
            let upgrade = ReadyUpgrade::new(protocol::PROTOCOL_NAME);
            return Poll::Ready(SubstreamProtocol::new(upgrade, ()));
        }
        Poll::Pending
    }
}

pub struct NewCircuitRequest {
    pub sender: oneshot::Sender<Result<Substream, StreamUpgradeError<Infallible>>>,
}

impl fmt::Debug for NewCircuitRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewCircuitRequest")
            .field("sender", &self.sender)
            .finish()
    }
}

struct CopyFuture<S, D> {
    src: BufReader<S>,
    dst: BufReader<D>,
}

impl<S, D> CopyFuture<S, D>
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(src: S, dst: D) -> Self {
        Self {
            src: BufReader::new(src),
            dst: BufReader::new(dst),
        }
    }
}

impl<S, D> Future for CopyFuture<S, D>
where
    S: AsyncRead + AsyncWrite + Unpin,
    D: AsyncRead + AsyncWrite + Unpin,
{
    type Output = io::Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        loop {
            enum Status {
                Pending,
                Done,
                Progressed,
            }
            let src_status = match forward_data(&mut this.src, &mut this.dst, cx) {
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => Status::Done,
                Poll::Ready(Ok(_)) => Status::Progressed,
                Poll::Pending => Status::Pending,
            };

            let dst_status = match forward_data(&mut this.dst, &mut this.src, cx) {
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(0)) => Status::Done,
                Poll::Ready(Ok(_)) => Status::Progressed,
                Poll::Pending => Status::Pending,
            };
            match (src_status, dst_status) {
                (Status::Done, Status::Done) => return Poll::Ready(Ok(())),
                (Status::Progressed, _) | (_, Status::Progressed) => {}
                (Status::Pending, Status::Pending) => {}
                (Status::Pending, Status::Done) | (Status::Done, Status::Pending) => {}
            }

            return Poll::Pending;
        }
    }
}

fn forward_data<S: AsyncBufRead + Unpin, D: AsyncWrite + Unpin>(
    mut src: &mut S,
    mut dst: &mut D,
    cx: &mut Context<'_>,
) -> Poll<io::Result<u64>> {
    let buffer = match Pin::new(&mut src).poll_fill_buf(cx)? {
        Poll::Ready(buffer) => buffer,
        Poll::Pending => {
            let _ = Pin::new(&mut dst).poll_flush(cx)?;
            return Poll::Pending;
        }
    };

    if buffer.is_empty() {
        ready!(Pin::new(&mut dst).poll_flush(cx))?;
        ready!(Pin::new(&mut dst).poll_close(cx))?;
        return Poll::Ready(Ok(0));
    }

    let i = ready!(Pin::new(dst).poll_write(cx, buffer))?;
    if i == 0 {
        return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
    }
    Pin::new(src).consume(i);

    Poll::Ready(Ok(i.try_into().expect("usize to fit into u64.")))
}

struct CircuitParts {
    src_stream: Substream,
    src_pending_data: Bytes,
    dst_peer_id: PeerId,
    dst_stream: Substream,
    dst_pending_data: Bytes,
}
