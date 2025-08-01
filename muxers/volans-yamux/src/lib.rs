use futures::{AsyncRead, AsyncWrite, future, ready};
use std::{
    collections::VecDeque,
    io, iter,
    pin::Pin,
    task::{Context, Poll, Waker},
};
use volans_core::{
    StreamMuxer, UpgradeInfo,
    upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade},
};

pub use yamux::{Config, Connection, ConnectionError, Mode, Stream};

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
        self.inbound_stream_waker = Some(cx.waker().clone());
        Poll::Pending
    }

    fn poll_outbound(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.as_mut().connection.poll_new_outbound(cx)
    }

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut this = self.as_mut();
        let inbound_stream = ready!(this.connection.poll_next_inbound(cx))
            .ok_or_else(|| ConnectionError::Closed)??;

        if this.inbound_stream_buffer.len() >= MAX_BUFFERED_INBOUND_STREAMS {
            tracing::warn!(
                "Inbound stream buffer is full, dropping stream: {}",
                inbound_stream.id()
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
pub struct UpgradeConfig(Config);

impl From<Config> for UpgradeConfig {
    fn from(config: Config) -> Self {
        UpgradeConfig(config)
    }
}

impl Default for UpgradeConfig {
    fn default() -> Self {
        UpgradeConfig(Config::default())
    }
}

impl UpgradeInfo for UpgradeConfig {
    type Info = &'static str;
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once("/v1/yamux")
    }
}

impl<C> InboundConnectionUpgrade<C> for UpgradeConfig
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Muxer<C>;
    type Error = io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, _: Self::Info) -> Self::Future {
        let connection = Connection::new(socket, self.0, Mode::Client);
        future::ready(Ok(Muxer::new(connection)))
    }
}

impl<C> OutboundConnectionUpgrade<C> for UpgradeConfig
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = Muxer<C>;
    type Error = io::Error;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, _: Self::Info) -> Self::Future {
        let connection = Connection::new(socket, self.0, Mode::Server);
        future::ready(Ok(Muxer::new(connection)))
    }
}
