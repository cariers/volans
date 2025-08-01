use bytes::{BufMut, Bytes, BytesMut};
use futures::{AsyncRead, AsyncWrite, Sink, Stream, ready};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use crate::length_delimited::{LengthDelimited, LengthDelimitedReader};

const MSG_PROTOCOL_NA: &[u8] = b"na";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Protocol(String);
impl AsRef<str> for Protocol {
    fn as_ref(&self) -> &str {
        self.0.as_ref()
    }
}

impl TryFrom<Bytes> for Protocol {
    type Error = ProtocolError;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        if !value.as_ref().starts_with(b"/") {
            return Err(ProtocolError::InvalidProtocol);
        }
        let protocol_as_string =
            String::from_utf8(value.to_vec()).map_err(|_| ProtocolError::InvalidProtocol)?;

        Ok(Protocol(protocol_as_string))
    }
}

impl TryFrom<&[u8]> for Protocol {
    type Error = ProtocolError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Self::try_from(Bytes::copy_from_slice(value))
    }
}

impl TryFrom<&str> for Protocol {
    type Error = ProtocolError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if !value.starts_with('/') {
            return Err(ProtocolError::InvalidProtocol);
        }

        Ok(Protocol(value.to_owned()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),
    #[error("Received an invalid message.")]
    InvalidMessage,
    #[error("A protocol (name) is invalid.")]
    InvalidProtocol,
}

impl From<ProtocolError> for io::Error {
    fn from(err: ProtocolError) -> Self {
        match err {
            ProtocolError::IoError(e) => e,
            ProtocolError::InvalidMessage => io::Error::new(io::ErrorKind::InvalidData, err),
            ProtocolError::InvalidProtocol => io::Error::new(io::ErrorKind::InvalidInput, err),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    Protocol(Protocol),
    NotAvailable,
}

impl Message {
    fn encode(&self, dst: &mut BytesMut) {
        match self {
            Message::NotAvailable => {
                dst.reserve(MSG_PROTOCOL_NA.len());
                dst.put(MSG_PROTOCOL_NA);
            }
            Message::Protocol(protocol) => {
                dst.reserve(protocol.as_ref().len());
                dst.put(protocol.0.as_ref());
            }
        }
    }

    fn decode(mut src: Bytes) -> Result<Self, ProtocolError> {
        if src == MSG_PROTOCOL_NA {
            return Ok(Message::NotAvailable);
        }
        if src.first() == Some(&b'/') {
            let protocol = Protocol::try_from(src.split_to(src.len()))?;
            return Ok(Message::Protocol(protocol));
        }
        Err(ProtocolError::InvalidMessage)
    }
}

#[pin_project::pin_project]
pub(crate) struct MessageIO<R> {
    #[pin]
    inner: LengthDelimited<R>,
}

impl<R> MessageIO<R> {
    pub(crate) fn new(inner: R) -> MessageIO<R>
    where
        R: AsyncRead + AsyncWrite,
    {
        Self {
            inner: LengthDelimited::new(inner),
        }
    }

    pub(crate) fn into_reader(self) -> MessageReader<R> {
        MessageReader {
            inner: self.inner.into_reader(),
        }
    }

    pub(crate) fn into_inner(self) -> R {
        self.inner.into_inner()
    }
}

impl<R> Sink<Message> for MessageIO<R>
where
    R: AsyncWrite,
{
    type Error = ProtocolError;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_ready(cx).map_err(From::from)
    }

    fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        let mut buf = BytesMut::new();
        item.encode(&mut buf);
        self.project()
            .inner
            .start_send(buf.freeze())
            .map_err(From::from)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_flush(cx).map_err(From::from)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().inner.poll_close(cx).map_err(From::from)
    }
}

impl<R> Stream for MessageIO<R>
where
    R: AsyncRead,
{
    type Item = Result<Message, ProtocolError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        poll_stream(self.project().inner, cx)
    }
}

#[pin_project::pin_project]
#[derive(Debug)]
pub(crate) struct MessageReader<R> {
    #[pin]
    inner: LengthDelimitedReader<R>,
}

impl<R> MessageReader<R> {
    pub(crate) fn into_inner(self) -> R {
        self.inner.into_inner()
    }
}

impl<R> Stream for MessageReader<R>
where
    R: AsyncRead,
{
    type Item = Result<Message, ProtocolError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        poll_stream(self.project().inner, cx)
    }
}

impl<R> AsyncWrite for MessageReader<R>
where
    R: AsyncWrite,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        self.project().inner.poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.project().inner.poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        self.project().inner.poll_close(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        self.project().inner.poll_write_vectored(cx, bufs)
    }
}

fn poll_stream<S>(
    stream: Pin<&mut S>,
    cx: &mut Context<'_>,
) -> Poll<Option<Result<Message, ProtocolError>>>
where
    S: Stream<Item = Result<Bytes, io::Error>>,
{
    let msg = if let Some(msg) = ready!(stream.poll_next(cx)?) {
        match Message::decode(msg) {
            Ok(m) => m,
            Err(err) => return Poll::Ready(Some(Err(err))),
        }
    } else {
        return Poll::Ready(None);
    };

    Poll::Ready(Some(Ok(msg)))
}
