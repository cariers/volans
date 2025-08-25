mod listener;
mod stream;

use std::{
    collections::VecDeque,
    io, mem,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    FutureExt, StreamExt, TryFutureExt,
    future::{self, BoxFuture, Ready},
};
use if_watch::IfEvent;
use volans_core::{
    Listener, ListenerEvent, Multiaddr, Transport, TransportError, multiaddr::Protocol,
};

pub use stream::TcpStream;
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct Config {
    ttl: Option<u32>,
    nodelay: bool,
    backlog: u32,
}

impl Config {
    pub fn new() -> Self {
        Self {
            ttl: None,
            nodelay: true,
            backlog: 1024,
        }
    }

    pub fn ttl(mut self, value: u32) -> Self {
        self.ttl = Some(value);
        self
    }

    pub fn nodelay(mut self, value: bool) -> Self {
        self.nodelay = value;
        self
    }

    pub fn listen_backlog(mut self, backlog: u32) -> Self {
        self.backlog = backlog;
        self
    }

    fn create_socket(&self, socket_addr: SocketAddr) -> io::Result<socket2::Socket> {
        let socket = socket2::Socket::new(
            socket2::Domain::for_address(socket_addr),
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        if socket_addr.is_ipv6() {
            socket.set_only_v6(true)?;
        }

        if let Some(ttl) = self.ttl {
            match socket_addr.is_ipv6() {
                true => {}
                false => socket.set_ttl_v4(ttl)?,
            }
        }
        socket.set_tcp_nodelay(self.nodelay)?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        Ok(socket)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for Config {
    type Output = TcpStream;
    type Error = io::Error;
    type Dial = BoxFuture<'static, Result<Self::Output, Self::Error>>;
    type Incoming = Ready<Result<Self::Output, Self::Error>>;
    type Listener = ListenStream;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let socket_addr = match multiaddr_to_socket_addr(addr.clone()) {
            Ok(socket) if socket.port() != 0 && !socket.ip().is_unspecified() => socket,
            _ => return Err(TransportError::NotSupported(addr)),
        };

        let fut = tokio::net::TcpStream::connect(socket_addr)
            .map_ok(TcpStream::from)
            .boxed();
        Ok(fut)
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        tracing::debug!("Listening for TCP connections on {}", addr);
        let socket_addr = match multiaddr_to_socket_addr(addr.clone()) {
            Ok(socket) => socket,
            _ => return Err(TransportError::NotSupported(addr)),
        };
        let socket = self.create_socket(socket_addr)?;
        socket.bind(&socket_addr.into())?;
        socket.listen(self.backlog as _)?;
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(socket.into())?;

        if socket_addr.ip().is_unspecified() {
            return Ok(ListenStream {
                listen_addr: socket_addr,
                pending_events: VecDeque::new(),
                state: State::Listening { listener },
                if_watcher: Some(if_watch::tokio::IfWatcher::new()?),
            });
        }
        let mut pending_events = VecDeque::new();
        pending_events.push_back(ListenerEvent::NewAddress(addr.clone()));

        Ok(ListenStream {
            listen_addr: socket_addr,
            pending_events,
            state: State::Listening { listener },
            if_watcher: None,
        })
    }
}

fn multiaddr_to_socket_addr(mut addr: Multiaddr) -> Result<SocketAddr, ()> {
    let mut port = None;
    while let Some(proto) = addr.pop() {
        match proto {
            Protocol::Ip4(ipv4) => match port {
                Some(port) => return Ok(SocketAddr::new(ipv4.into(), port)),
                None => return Err(()),
            },
            Protocol::Ip6(ipv6) => match port {
                Some(port) => return Ok(SocketAddr::new(ipv6.into(), port)),
                None => return Err(()),
            },
            Protocol::Tcp(port_num) => match port {
                Some(_) => return Err(()),
                None => port = Some(port_num),
            },
            Protocol::Peer(_) => {}
            _ => return Err(()),
        }
    }
    Err(())
}

pub struct ListenStream {
    listen_addr: SocketAddr,
    pending_events: VecDeque<ListenerEvent<Ready<Result<TcpStream, io::Error>>, io::Error>>,
    state: State,
    if_watcher: Option<if_watch::tokio::IfWatcher>,
}

enum State {
    Listening { listener: TcpListener },
    Closed,
}

impl ListenStream {
    // fn poll_if_watch(self: Pin<&mut Self>, cx: &mut Context<'_>) {
    //     if
    // }
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
        if let Some(event) = this.pending_events.pop_front() {
            return Poll::Ready(event);
        }

        if let Some(if_watcher) = this.if_watcher.as_mut() {
            while let Poll::Ready(Some(if_event)) = if_watcher.poll_next_unpin(cx) {
                match if_event {
                    Ok(IfEvent::Up(inet)) => {
                        let ip = inet.addr();
                        if this.listen_addr.is_ipv4() == ip.is_ipv4() {
                            let addr = ip_to_multiaddr(ip, this.listen_addr.port());
                            return Poll::Ready(ListenerEvent::NewAddress(addr));
                        }
                    }
                    Ok(IfEvent::Down(inet)) => {
                        let ip = inet.addr();
                        if this.listen_addr.is_ipv4() == ip.is_ipv4() {
                            let addr = ip_to_multiaddr(ip, this.listen_addr.port());
                            return Poll::Ready(ListenerEvent::AddressExpired(addr));
                        }
                    }
                    Err(err) => return Poll::Ready(ListenerEvent::Error(err)),
                }
            }
        }

        match &mut this.state {
            State::Listening { listener } => match Pin::new(listener).poll_accept(cx) {
                Poll::Ready(Ok((stream, remote_addr))) => {
                    let local_addr = match stream.local_addr() {
                        Ok(addr) => addr,
                        Err(e) => return Poll::Ready(ListenerEvent::Error(e)),
                    };
                    let upgrade = future::ok(TcpStream::from(stream));
                    let local_addr = ip_to_multiaddr(local_addr.ip(), local_addr.port());
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
