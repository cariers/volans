use futures::{AsyncRead, AsyncWrite, Sink, Stream};

use crate::{
    Negotiated, NegotiationError,
    protocol::{Message, MessageIO, Protocol},
};
use std::{
    iter, mem,
    pin::Pin,
    task::{Context, Poll},
};

#[pin_project::pin_project]
pub struct DialerSelectFuture<R, I: Iterator> {
    protocols: iter::Peekable<I>,
    state: State<R, I::Item>,
}

impl<R, I> DialerSelectFuture<R, I>
where
    R: AsyncRead + AsyncWrite,
    I: Iterator,
    I::Item: AsRef<str>,
{
    pub fn new(io: R, protocols: I) -> Self {
        DialerSelectFuture {
            protocols: protocols.peekable(),
            state: State::Initial {
                io: MessageIO::new(io),
            },
        }
    }
}

enum State<R, P> {
    Initial { io: MessageIO<R> },
    SendProtocol { io: MessageIO<R>, protocol: P },
    FlushProtocol { io: MessageIO<R>, protocol: P },
    AwaitProtocol { io: MessageIO<R>, protocol: P },
    Done,
}

impl<R, I> Future for DialerSelectFuture<R, I>
where
    R: AsyncRead + AsyncWrite + Unpin,
    I: Iterator,
    I::Item: AsRef<str>,
{
    type Output = Result<(I::Item, Negotiated<R>), NegotiationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        loop {
            match mem::replace(this.state, State::Done) {
                State::Initial { mut io } => {
                    match Pin::new(&mut io).poll_ready(cx)? {
                        Poll::Ready(()) => {}
                        Poll::Pending => {
                            *this.state = State::Initial { io };
                            return Poll::Pending;
                        }
                    };
                    let protocol = this.protocols.next().ok_or(NegotiationError::Failed)?;
                    *this.state = State::SendProtocol { io, protocol };
                }
                State::SendProtocol { mut io, protocol } => {
                    tracing::trace!("Sending protocol: {}", protocol.as_ref());
                    match Pin::new(&mut io).poll_ready(cx)? {
                        Poll::Ready(()) => {}
                        Poll::Pending => {
                            *this.state = State::SendProtocol { io, protocol };
                            return Poll::Pending;
                        }
                    };
                    let p = Protocol::try_from(protocol.as_ref())?;
                    // 发送协议到 IO
                    if let Err(err) = Pin::new(&mut io).start_send(Message::Protocol(p.clone())) {
                        return Poll::Ready(Err(From::from(err)));
                    }
                    if this.protocols.peek().is_some() {
                        // 如果还有更多协议，进入发送协议状态
                        *this.state = State::FlushProtocol { io, protocol };
                    } else {
                        // 如果没有更多协议，直接进入等待状态
                        tracing::trace!("Expecting protocol: {}", p.as_ref());
                        let io = Negotiated::expecting(io.into_reader(), p);
                        return Poll::Ready(Ok((protocol, io)));
                    }
                }
                State::FlushProtocol { mut io, protocol } => {
                    match Pin::new(&mut io).poll_flush(cx)? {
                        Poll::Ready(()) => {}
                        Poll::Pending => {
                            *this.state = State::FlushProtocol { io, protocol };
                            return Poll::Pending;
                        }
                    };
                    // 进入等待状态
                    *this.state = State::AwaitProtocol { io, protocol };
                }
                State::AwaitProtocol { mut io, protocol } => {
                    let msg = match Pin::new(&mut io).poll_next(cx)? {
                        Poll::Ready(Some(msg)) => msg,
                        Poll::Ready(None) => {
                            tracing::debug!("No message received, connection closed");
                            return Poll::Ready(Err(NegotiationError::Failed));
                        }
                        Poll::Pending => {
                            *this.state = State::AwaitProtocol { io, protocol };
                            return Poll::Pending;
                        }
                    };
                    match msg {
                        Message::Protocol(p) if p.as_ref() == protocol.as_ref() => {
                            // 协议匹配成功，返回 Negotiated
                            let io = Negotiated::completed(io.into_inner());
                            return Poll::Ready(Ok((protocol, io)));
                        }
                        Message::NotAvailable => {
                            // 不支持的协议，继续协商下一个协议
                            tracing::debug!("Protocol not available, trying next protocol");
                            let protocol = this.protocols.next().ok_or(NegotiationError::Failed)?;
                            *this.state = State::SendProtocol { io, protocol }
                        }
                        _ => {
                            // 协议不匹配，继续等待下一个协议
                            *this.state = State::Initial { io };
                        }
                    }
                }
                _ => panic!("Unexpected state in DialerSelectFuture"),
            }
        }
    }
}
