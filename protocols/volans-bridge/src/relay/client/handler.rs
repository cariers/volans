use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    AsyncBufRead, AsyncRead, AsyncWrite, channel::oneshot, io::BufReader, ready,
    stream::FuturesUnordered,
};
use volans_core::upgrade::ReadyUpgrade;
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol,
};

use crate::{PROTOCOL_NAME, relay::CircuitRequest};

/// 中继服务器处理连接Backend的请求
pub struct Handler {
    requested_streams: VecDeque<CircuitRequest>,
    /// 待向Backend发送的请求
    pending_streams: VecDeque<CircuitRequest>,
    worker_stream: FuturesUnordered<CopyFuture<Substream, Substream>>,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            requested_streams: VecDeque::new(),
            pending_streams: VecDeque::new(),
            worker_stream: FuturesUnordered::new(),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = CircuitRequest;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        // 待向Backend发送的请求
        self.requested_streams.push_back(action);
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
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
            dst_peer_id,
            src_stream,
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
        let fut = CopyFuture::new(src_stream, stream);
        self.worker_stream.push(fut);
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
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if let Some(request) = self.requested_streams.pop_front() {
            // 将请求发送到待处理的流
            self.pending_streams.push_back(request);
            let upgrade = ReadyUpgrade::new(PROTOCOL_NAME);
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
