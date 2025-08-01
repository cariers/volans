mod listener;
mod stream;

use std::{io, net::SocketAddr};

use volans_core::{Transport, TransportError, Url};
use futures::{
    FutureExt, TryFutureExt,
    future::{BoxFuture, Ready},
};

pub use listener::ListenStream;
use socket2::{Domain, Protocol, Socket, Type};
pub use stream::TcpStream;
use tokio::net::TcpListener;
use url::Host;

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

    fn create_socket(&self, socket_addr: SocketAddr) -> io::Result<Socket> {
        let socket = Socket::new(
            Domain::for_address(socket_addr),
            Type::STREAM,
            Some(Protocol::TCP),
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

    fn dial(&self, addr: &Url) -> Result<Self::Dial, TransportError<Self::Error>> {
        let addr = parse_url(addr)?;
        let fut = tokio::net::TcpStream::connect(addr)
            .map_ok(TcpStream::from)
            .boxed();
        Ok(fut)
    }

    fn listen(&self, addr: &Url) -> Result<Self::Listener, TransportError<Self::Error>> {
        let socket_addr = parse_url(addr)?;
        let socket = self.create_socket(socket_addr)?;
        socket.bind(&socket_addr.into())?;
        socket.listen(self.backlog as _)?;
        socket.set_nonblocking(true)?;
        let listener = TcpListener::from_std(socket.into())?;
        Ok(ListenStream::new(listener, addr.clone()))
    }
}

fn parse_url(url: &Url) -> Result<SocketAddr, io::Error> {
    match (url.host(), url.port()) {
        (Some(Host::Ipv4(addr)), Some(port)) => Ok(SocketAddr::new(addr.into(), port)),
        (Some(Host::Ipv6(addr)), Some(port)) => Ok(SocketAddr::new(addr.into(), port)),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL must contain a host",
        )),
    }
}
