use futures::{AsyncRead, AsyncWrite, Sink, TryStream, ready};
use std::{
    io::{self, Read},
    pin::Pin,
    task::{Context, Poll},
};

#[pin_project::pin_project]
pub struct RwStreamSink<S: TryStream> {
    #[pin]
    inner: S,
    current_item: Option<std::io::Cursor<<S as TryStream>::Ok>>,
}

impl<S: TryStream> RwStreamSink<S> {
    /// Wraps around `inner`.
    pub fn new(inner: S) -> Self {
        RwStreamSink {
            inner,
            current_item: None,
        }
    }
}

impl<S> AsyncRead for RwStreamSink<S>
where
    S: TryStream<Error = io::Error>,
    <S as TryStream>::Ok: AsRef<[u8]>,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let mut this = self.project();

        // Grab the item to copy from.
        let item_to_copy = loop {
            if let Some(i) = this.current_item {
                if i.position() < i.get_ref().as_ref().len() as u64 {
                    break i;
                }
            }
            *this.current_item = Some(match ready!(this.inner.as_mut().try_poll_next(cx)) {
                Some(Ok(i)) => std::io::Cursor::new(i),
                Some(Err(e)) => return Poll::Ready(Err(e)),
                None => return Poll::Ready(Ok(0)), // EOF
            });
        };

        // Copy it!
        Poll::Ready(Ok(item_to_copy.read(buf)?))
    }
}

impl<S> AsyncWrite for RwStreamSink<S>
where
    S: TryStream + Sink<<S as TryStream>::Ok, Error = io::Error>,
    <S as TryStream>::Ok: for<'r> From<&'r [u8]>,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context, buf: &[u8]) -> Poll<io::Result<usize>> {
        let mut this = self.project();
        ready!(this.inner.as_mut().poll_ready(cx)?);
        let n = buf.len();
        if let Err(e) = this.inner.start_send(buf.into()) {
            return Poll::Ready(Err(e));
        }
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let this = self.project();
        this.inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        let this = self.project();
        this.inner.poll_close(cx)
    }
}
