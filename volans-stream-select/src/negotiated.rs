use crate::{
    ProtocolError,
    protocol::{Message, MessageReader, Protocol},
};
use futures::{AsyncRead, AsyncWrite, Stream, ready};
use pin_project::pin_project;
use std::{
    io, mem,
    pin::Pin,
    task::{Context, Poll},
};

#[pin_project]
#[derive(Debug)]
pub struct Negotiated<R> {
    #[pin]
    state: State<R>,
}

impl<R> Negotiated<R> {
    pub(crate) fn completed(io: R) -> Self {
        Negotiated {
            state: State::Completed { io },
        }
    }

    pub(crate) fn expecting(io: MessageReader<R>, protocol: Protocol) -> Self {
        Negotiated {
            state: State::Expecting { io, protocol },
        }
    }

    pub fn complete(self) -> NegotiatedComplete<R> {
        NegotiatedComplete { inner: Some(self) }
    }
}

#[pin_project(project = StateProj)]
#[derive(Debug)]
enum State<R> {
    Expecting {
        #[pin]
        io: MessageReader<R>,
        protocol: Protocol,
    },
    Completed {
        #[pin]
        io: R,
    },

    Invalid,
}

impl<R> Negotiated<R> {
    fn poll_negotiated(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), NegotiationError>>
    where
        R: AsyncRead + AsyncWrite + Unpin,
    {
        match self.as_mut().poll_flush(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(e)) => {
                if e.kind() != io::ErrorKind::WriteZero {
                    return Poll::Ready(Err(e.into()));
                }
            }
        }
        let mut this = self.project();
        if let StateProj::Completed { .. } = this.state.as_mut().project() {
            return Poll::Ready(Ok(()));
        }
        loop {
            match mem::replace(&mut *this.state, State::Invalid) {
                State::Expecting { mut io, protocol } => {
                    let msg = match Pin::new(&mut io).poll_next(cx)? {
                        Poll::Ready(Some(msg)) => msg,
                        Poll::Ready(None) => {
                            return Poll::Ready(Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "unexpected end of stream",
                            )
                            .into()));
                        }
                        Poll::Pending => {
                            *this.state = State::Expecting { io, protocol };
                            return Poll::Pending;
                        }
                    };
                    tracing::trace!("Received message: {:?}", msg);
                    if let Message::Protocol(p) = &msg {
                        if p.as_ref() == protocol.as_ref() {
                            tracing::trace!("Negotiated protocol completed: {}", p.as_ref());
                            *this.state = State::Completed {
                                io: io.into_inner(),
                            };
                            return Poll::Ready(Ok(()));
                        }
                    }
                    return Poll::Ready(Err(NegotiationError::Failed));
                }
                _ => panic!("Negotiated state should not be in Invalid state"),
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NegotiationError {
    #[error("Invalid Protocol, {0}")]
    ProtocolError(#[from] ProtocolError),
    #[error("Protocol negotiation failed.")]
    Failed,
}

impl From<io::Error> for NegotiationError {
    fn from(err: io::Error) -> NegotiationError {
        ProtocolError::from(err).into()
    }
}

impl From<NegotiationError> for io::Error {
    fn from(err: NegotiationError) -> io::Error {
        if let NegotiationError::ProtocolError(e) = err {
            return e.into();
        }
        io::Error::other(err)
    }
}

impl<R> AsyncRead for Negotiated<R>
where
    R: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            if let StateProj::Completed { io } = self.as_mut().project().state.project() {
                return io.poll_read(cx, buf);
            }
            match self.as_mut().poll_negotiated(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e.into())),
            }
        }
    }

    fn poll_read_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        loop {
            if let StateProj::Completed { io } = self.as_mut().project().state.project() {
                return io.poll_read_vectored(cx, bufs);
            }
            //
            match self.as_mut().poll_negotiated(cx) {
                Poll::Ready(Ok(())) => {}
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e.into())),
            }
        }
    }
}

impl<R> AsyncWrite for Negotiated<R>
where
    R: AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.project().state.project() {
            StateProj::Completed { io } => io.poll_write(cx, buf),
            StateProj::Expecting { io, .. } => io.poll_write(cx, buf),
            StateProj::Invalid => panic!("Negotiated state should not be in Invalid state"),
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        match self.project().state.project() {
            StateProj::Completed { io } => io.poll_write_vectored(cx, bufs),
            StateProj::Expecting { io, .. } => io.poll_write_vectored(cx, bufs),
            StateProj::Invalid => panic!("Negotiated state should not be in Invalid state"),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.project().state.project() {
            StateProj::Completed { io } => io.poll_flush(cx),
            StateProj::Expecting { io, .. } => io.poll_flush(cx),
            StateProj::Invalid => panic!("Negotiated state should not be in Invalid state"),
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.as_mut().poll_flush(cx))?;
        match self.project().state.project() {
            StateProj::Completed { io } => io.poll_close(cx),
            StateProj::Expecting { io, .. } => io.poll_close(cx),
            StateProj::Invalid => panic!("Negotiated state should not be in Invalid state"),
        }
    }
}

#[derive(Debug)]
pub struct NegotiatedComplete<R> {
    inner: Option<Negotiated<R>>,
}

impl<R> Future for NegotiatedComplete<R>
where
    R: AsyncRead + AsyncWrite + Unpin,
{
    type Output = Result<Negotiated<R>, NegotiationError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut io = self
            .inner
            .take()
            .expect("NegotiatedFuture called after completion.");
        match Pin::new(&mut io).poll_negotiated(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(io)),
            Poll::Pending => {
                self.get_mut().inner = Some(io);
                Poll::Pending
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }
}
