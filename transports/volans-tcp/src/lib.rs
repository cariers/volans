mod listener;
mod stream;

use std::{io, net::SocketAddr};

use futures::{
    FutureExt, TryFutureExt,
    future::{BoxFuture, Ready},
};
use volans_core::{Multiaddr, Transport, TransportError, multiaddr::Protocol};

pub use listener::ListenStream;
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
            socket.set_ttl(ttl)?;
        }
        socket.set_nodelay(self.nodelay)?;
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
        let socket_addr = match multiaddr_to_socket_addr(addr.clone()) {
            Ok(socket) => socket,
            _ => return Err(TransportError::NotSupported(addr)),
        };
        let socket = self.create_socket(socket_addr)?;
        socket.bind(&socket_addr.into())?;
        socket.listen(self.backlog as _)?;
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(socket.into())?;
        Ok(ListenStream::new(listener, socket_addr))
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
