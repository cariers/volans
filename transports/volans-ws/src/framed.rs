use async_tungstenite::{WebSocketStream, tungstenite};
use futures::{AsyncRead, AsyncWrite, Sink, Stream, ready};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

pub struct BytesWebSocketStream<C> {
    inner: WebSocketStream<C>,
}

impl<C> BytesWebSocketStream<C> {
    pub(crate) fn new(inner: WebSocketStream<C>) -> Self {
        Self { inner }
    }
}

impl<C> Stream for BytesWebSocketStream<C>
where
    C: AsyncRead + AsyncWrite + Unpin,
{
    type Item = io::Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
                Some(Ok(tungstenite::Message::Binary(data))) => {
                    return Poll::Ready(Some(Ok(data.into())));
                }
                None => {
                    return Poll::Ready(None);
                }
                Some(Ok(_)) => {}
                Some(Err(err)) => {
                    return Poll::Ready(Some(Err(into_io_error(err))));
                }
            }
        }
    }
}

impl<C> Sink<Vec<u8>> for BytesWebSocketStream<C>
where
    C: AsyncRead + AsyncWrite + Unpin,
{
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_ready(cx)
            .map_err(into_io_error)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        Pin::new(&mut self.inner)
            .start_send(tungstenite::Message::Binary(item.into()))
            .map_err(into_io_error)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_flush(cx)
            .map_err(into_io_error)
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(&mut self.inner)
            .poll_close(cx)
            .map_err(into_io_error)
    }
}

fn into_io_error(error: tungstenite::Error) -> io::Error {
    match error {
        tungstenite::Error::Io(e) => e,
        e => io::Error::new(io::ErrorKind::Other, e),
    }
}
