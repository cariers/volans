use futures::{AsyncRead, AsyncWrite, Sink, Stream};
use smallvec::SmallVec;

use crate::{
    Negotiated, NegotiationError, ProtocolError,
    protocol::{Message, MessageIO, Protocol},
};
use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};

#[pin_project::pin_project]
pub struct ListenerSelectFuture<R, N> {
    // 使用 smallvec, 在堆上分配内存之前，它会在栈上存储一定数量的元素。
    protocols: SmallVec<[(N, Protocol); 8]>,
    state: State<R, N>,
}

impl<R, N> ListenerSelectFuture<R, N>
where
    R: AsyncRead + AsyncWrite + Unpin,
    N: AsRef<str> + Clone,
{
    pub fn new<I>(io: R, protocols: I) -> Self
    where
        I: Iterator<Item = N>,
    {
        let protocols =
            protocols
                .into_iter()
                .filter_map(|n| match Protocol::try_from(n.as_ref()) {
                    Ok(p) => Some((n, p)),
                    Err(_) => None,
                });

        ListenerSelectFuture {
            protocols: SmallVec::from_iter(protocols),
            state: State::RecvMessage {
                io: MessageIO::new(io),
            },
        }
    }
}

enum State<R, N> {
    RecvMessage {
        io: MessageIO<R>,
    },
    SendMessage {
        io: MessageIO<R>,
        message: Message,
        protocol: Option<N>,
    },
    Flush {
        io: MessageIO<R>,
        protocol: Option<N>,
    },
    Done,
}

impl<R, N> Future for ListenerSelectFuture<R, N>
where
    R: AsyncRead + AsyncWrite + Unpin,
    N: AsRef<str> + Clone,
{
    type Output = Result<(N, Negotiated<R>), NegotiationError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        loop {
            match mem::replace(this.state, State::Done) {
                State::RecvMessage { mut io } => {
                    let msg = match Pin::new(&mut io).poll_next(cx)? {
                        Poll::Ready(Some(msg)) => msg,
                        Poll::Ready(None) => {
                            return Poll::Ready(Err(NegotiationError::Failed));
                        }
                        Poll::Pending => {
                            *this.state = State::RecvMessage { io };
                            return Poll::Pending;
                        }
                    };
                    tracing::trace!("Received message: {:?}", msg);
                    match msg {
                        Message::Protocol(p) => {
                            // 查找匹配的协议
                            let protocol = this.protocols.iter().find_map(|(name, proto)| {
                                if &p == proto {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            });
                            let message = if protocol.is_some() {
                                Message::Protocol(p.clone())
                            } else {
                                Message::NotAvailable
                            };
                            *this.state = State::SendMessage {
                                io,
                                message,
                                protocol,
                            };
                        }
                        _ => return Poll::Ready(Err(ProtocolError::InvalidMessage.into())),
                    }
                }
                State::SendMessage {
                    mut io,
                    message,
                    protocol,
                } => {
                    match Pin::new(&mut io).poll_ready(cx)? {
                        Poll::Ready(()) => {}
                        Poll::Pending => {
                            *this.state = State::SendMessage {
                                io,
                                message,
                                protocol,
                            };
                            return Poll::Pending;
                        }
                    };
                    if let Err(err) = Pin::new(&mut io).start_send(message) {
                        return Poll::Ready(Err(From::from(err)));
                    }
                    *this.state = State::Flush { io, protocol };
                }
                State::Flush { mut io, protocol } => {
                    match Pin::new(&mut io).poll_flush(cx)? {
                        Poll::Ready(()) => {}
                        Poll::Pending => {
                            *this.state = State::Flush { io, protocol };
                            return Poll::Pending;
                        }
                    };
                    if let Some(protocol) = protocol {
                        // 协议匹配成功，返回 Negotiated
                        let io = Negotiated::completed(io.into_inner());
                        tracing::trace!(
                            "Negotiation successful for protocol: {}",
                            protocol.as_ref()
                        );
                        return Poll::Ready(Ok((protocol, io)));
                    } else {
                        // 如果没有匹配的协议，继续接收消息
                        *this.state = State::RecvMessage { io }
                    }
                }
                _ => panic!("Unexpected state in ListenerSelectFuture"),
            }
        }
    }
}
