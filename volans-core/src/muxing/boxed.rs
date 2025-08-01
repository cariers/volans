use crate::StreamMuxer;
use futures::{AsyncRead, AsyncWrite};
use pin_project::pin_project;
use std::{
    error::Error,
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

pub struct StreamMuxerBox {
    inner: Pin<Box<dyn StreamMuxer<Substream = SubstreamBox, Error = io::Error> + Send>>,
}

impl fmt::Debug for StreamMuxerBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamMuxerBox").finish_non_exhaustive()
    }
}

#[pin_project]
struct Wrap<T>
where
    T: StreamMuxer,
{
    #[pin]
    inner: T,
}

impl<T> StreamMuxer for Wrap<T>
where
    T: StreamMuxer + Send + 'static,
    T::Substream: Send + 'static,
    T::Error: Send + Sync + 'static,
{
    type Substream = SubstreamBox;
    type Error = io::Error;

    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.project()
            .inner
            .poll_inbound(cx)
            .map_ok(SubstreamBox::new)
            .map_err(into_io_error)
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.project()
            .inner
            .poll_outbound(cx)
            .map_ok(SubstreamBox::new)
            .map_err(into_io_error)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx).map_err(into_io_error)
    }

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll(cx).map_err(into_io_error)
    }
}

impl StreamMuxer for StreamMuxerBox {
    type Substream = SubstreamBox;
    type Error = io::Error;

    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.get_mut().inner.as_mut().poll_inbound(cx)
    }

    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>> {
        self.get_mut().inner.as_mut().poll_outbound(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().inner.as_mut().poll_close(cx)
    }

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().inner.as_mut().poll(cx)
    }
}

fn into_io_error<E>(err: E) -> io::Error
where
    E: Error + Send + Sync + 'static,
{
    io::Error::other(err)
}

impl StreamMuxerBox {
    pub fn new<T>(muxer: T) -> StreamMuxerBox
    where
        T: StreamMuxer + Send + 'static,
        T::Substream: Send + 'static,
        T::Error: Send + Sync + 'static,
    {
        let wrap = Wrap { inner: muxer };
        StreamMuxerBox {
            inner: Box::pin(wrap),
        }
    }
}

pub struct SubstreamBox(Pin<Box<dyn AsyncReadWrite + Send>>);

impl SubstreamBox {
    pub fn new<S>(substream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        SubstreamBox(Box::pin(substream))
    }
}

impl fmt::Debug for SubstreamBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamMuxerBox({})", self.0.type_name())
    }
}

trait AsyncReadWrite: AsyncRead + AsyncWrite {
    fn type_name(&self) -> &'static str;
}

impl<S> AsyncReadWrite for S
where
    S: AsyncRead + AsyncWrite,
{
    fn type_name(&self) -> &'static str {
        std::any::type_name::<S>()
    }
}

impl AsyncRead for SubstreamBox {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_read(cx, buf)
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_read_vectored(cx, bufs)
    }
}

impl AsyncWrite for SubstreamBox {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        self.0.as_mut().poll_write_vectored(cx, bufs)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_flush(cx)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.as_mut().poll_close(cx)
    }
}
