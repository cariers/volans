use volans_core::{Negotiated, muxing::SubstreamBox};
use either::Either;
use futures::{AsyncRead, AsyncWrite};

use std::{
    fmt,
    hash::{Hash, Hasher},
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

#[derive(Debug, Clone)]
pub(crate) struct ActiveStreamCounter(Arc<()>);

impl ActiveStreamCounter {
    pub(crate) fn new() -> Self {
        Self(Arc::new(()))
    }

    pub(crate) fn no_active_streams(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

#[derive(Debug)]
pub struct Substream {
    stream: Negotiated<SubstreamBox>,
    counter: Option<ActiveStreamCounter>,
}

impl Substream {
    pub(crate) fn new(stream: Negotiated<SubstreamBox>, counter: ActiveStreamCounter) -> Self {
        Self {
            stream,
            counter: Some(counter),
        }
    }

    pub fn ignore_for_keep_alive(&mut self) {
        self.counter.take();
    }
}

impl AsyncRead for Substream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_read_vectored(cx, bufs)
    }
}

impl AsyncWrite for Substream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_close(cx)
    }
}

#[derive(Clone, Eq)]
pub struct StreamProtocol {
    inner: Either<&'static str, Arc<str>>,
}

impl StreamProtocol {
    pub const fn new(s: &'static str) -> Self {
        match s.as_bytes() {
            [b'/', ..] => {}
            _ => panic!("Protocols should start with a /"),
        }

        StreamProtocol {
            inner: Either::Left(s),
        }
    }
    pub fn try_from_owned(protocol: String) -> Result<Self, InvalidProtocol> {
        if !protocol.starts_with('/') {
            return Err(InvalidProtocol::missing_forward_slash());
        }

        Ok(StreamProtocol {
            // FIXME: Can we somehow reuse the
            // allocation from the owned string?
            inner: Either::Right(Arc::from(protocol)),
        })
    }
}

impl AsRef<str> for StreamProtocol {
    fn as_ref(&self) -> &str {
        either::for_both!(&self.inner, s => s)
    }
}

impl fmt::Debug for StreamProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        either::for_both!(&self.inner, s => s.fmt(f))
    }
}

impl fmt::Display for StreamProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl PartialEq<&str> for StreamProtocol {
    fn eq(&self, other: &&str) -> bool {
        self.as_ref() == *other
    }
}

impl PartialEq<StreamProtocol> for &str {
    fn eq(&self, other: &StreamProtocol) -> bool {
        *self == other.as_ref()
    }
}

impl PartialEq for StreamProtocol {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Hash for StreamProtocol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state)
    }
}

#[derive(Debug)]
pub struct InvalidProtocol {
    _private: (),
}

impl InvalidProtocol {
    pub(crate) fn missing_forward_slash() -> Self {
        InvalidProtocol { _private: () }
    }
}
