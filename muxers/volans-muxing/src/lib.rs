use volans_core::{
    StreamMuxer, UpgradeInfo,
    upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade},
};
use futures::{AsyncRead, AsyncWrite, future, ready};
pub use muxing::{Connection, ConnectionError, Endpoint, Stream};
use std::{
    collections::VecDeque,
    io, iter,
    pin::Pin,
    task::{Context, Poll, Waker},
};

#[derive(Debug)]
pub struct Muxer<C> {
    connection: Connection<C>,
    inbound_stream_buffer: VecDeque<Stream>,
    inbound_stream_waker: Option<Waker>,
}

impl<C> Muxer<C>
where
    C: AsyncRead + AsyncWrite + Unpin + 'static,
{
    pub fn new(connection: Connection<C>) -> Self {
        Muxer {
            connection,
            inbound_stream_buffer: VecDeque::with_capacity(MAX_BUFFERED_INBOUND_STREAMS),
            inbound_stream_waker: None,
        }
    }

    fn poll_inner(&mut self, cx: &mut Context<'_>) -> Poll<Result<Stream, ConnectionError>> {
        let stream =
            ready!(self.connection.poll_next_inbound(cx)?).ok_or(ConnectionError::Closed)?;
        Poll::Ready(Ok(stream))
    }
}

const MAX_BUFFERED_INBOUND_STREAMS: usize = 256;

impl<C> StreamMuxer for Muxer<C>
where
    C: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Substream = Stream;
    type Error = ConnectionError;

    fn poll_inbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        if let Some(stream) = self.inbound_stream_buffer.pop_front() {
            return Poll::Ready(Ok(stream));
        }
        if let Poll::Ready(res) = self.poll_inner(cx) {
            return Poll::Ready(res);
        }
        self.inbound_stream_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.as_mut().connection.poll_new_outbound(cx)
    }

    #[tracing::instrument(level = "trace", name = "StreamMuxer::poll", skip(self, cx))]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut this = self.as_mut();
        let inbound_stream = ready!(this.poll_inner(cx))?;

        if this.inbound_stream_buffer.len() >= MAX_BUFFERED_INBOUND_STREAMS {
            tracing::warn!(
                "{}: Inbound stream buffer is full, dropping stream:",
                inbound_stream
            );
            drop(inbound_stream);
        } else {
            this.inbound_stream_buffer.push_back(inbound_stream);
            if let Some(waker) = this.inbound_stream_waker.take() {
                waker.wake();
            }
        }
        // 马上唤醒任务
        cx.waker().wake_by_ref();
        Poll::Pending
    }

    #[tracing::instrument(level = "trace", name = "StreamMuxer::poll_close", skip(self, cx))]
    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.as_mut().connection.poll_close(cx)
    }
}

#[derive(Debug, Clone)]
pub struct Config(muxing::Config);

impl Config {
    pub fn new() -> Self {
        Config(muxing::Config::default())
    }

    pub fn set_max_active_streams(&mut self, max_active_streams: usize) -> &mut Self {
        self.0.set_max_active_streams(max_active_streams);
        self
    }

    pub fn set_read_after_close(&mut self, read_after_close: bool) -> &mut Self {
        self.0.set_read_after_close(read_after_close);
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Config(muxing::Config::default())
    }
}

impl UpgradeInfo for Config {
    type Info = &'static str;
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once("/v1/muxing")
    }
}

impl<C> InboundConnectionUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Output = Muxer<C>;
    type Error = io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, _info: Self::Info) -> Self::Future {
        let connection = Connection::new(socket, self.0, Endpoint::Server);
        future::ready(Ok(Muxer::new(connection)))
    }
}

impl<C> OutboundConnectionUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Unpin + 'static,
{
    type Output = Muxer<C>;
    type Error = io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, _info: Self::Info) -> Self::Future {
        let connection = Connection::new(socket, self.0, Endpoint::Client);
        future::ready(Ok(Muxer::new(connection)))
    }
}
