use futures::future::{self, Ready};
use std::{
    io, mem,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
};
use tokio::net::TcpListener;
use volans_core::{Listener, ListenerEvent, Multiaddr, multiaddr::Protocol};

use crate::TcpStream;

pub struct ListenStream {
    listen_addr: SocketAddr,
    pending_event: Option<ListenerEvent<Ready<Result<TcpStream, io::Error>>, io::Error>>,
    state: State,
}

enum State {
    Listening { listener: TcpListener },
    Closed,
}

impl ListenStream {
    pub fn new(listener: TcpListener, listen_addr: SocketAddr) -> Self {
        let addr = ip_to_multiaddr(listen_addr.ip(), listen_addr.port());
        let listened_event = ListenerEvent::Listened(addr);
        ListenStream {
            listen_addr,
            state: State::Listening { listener },
            pending_event: Some(listened_event),
        }
    }
}

impl Listener for ListenStream {
    type Error = io::Error;
    type Output = TcpStream;
    type Upgrade = Ready<Result<TcpStream, io::Error>>;

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match mem::replace(&mut this.state, State::Closed) {
            State::Listening { listener } => {
                this.state = State::Closed;
                drop(listener);
                Poll::Ready(Ok(()))
            }
            State::Closed => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Listener closed")))
            }
        }
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.get_mut();
        if let Some(event) = this.pending_event.take() {
            return Poll::Ready(event);
        }
        match &mut this.state {
            State::Listening { listener } => match Pin::new(listener).poll_accept(cx) {
                Poll::Ready(Ok((stream, remote_addr))) => {
                    let upgrade = future::ok(TcpStream::from(stream));

                    let local_addr =
                        ip_to_multiaddr(this.listen_addr.ip(), this.listen_addr.port());
                    let remote_addr = ip_to_multiaddr(remote_addr.ip(), remote_addr.port());

                    let event = ListenerEvent::Incoming {
                        local_addr,
                        remote_addr,
                        upgrade,
                    };
                    Poll::Ready(event)
                }
                Poll::Ready(Err(e)) => {
                    let event = ListenerEvent::Error(e);
                    Poll::Ready(event)
                }
                Poll::Pending => return Poll::Pending,
            },
            State::Closed => Poll::Ready(ListenerEvent::Closed(Ok(()))),
        }
    }
}

fn ip_to_multiaddr(ip: IpAddr, port: u16) -> Multiaddr {
    Multiaddr::empty().with(ip.into()).with(Protocol::Tcp(port))
}
