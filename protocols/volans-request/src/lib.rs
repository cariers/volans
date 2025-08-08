pub mod codec;

pub mod client;
pub mod server;

use std::{
    convert::Infallible,
    fmt, io,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

pub use codec::Codec;
use futures::{channel::oneshot, future};
use smallvec::SmallVec;
use volans_core::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};
use volans_swarm::Substream;

const NEXT_REQUEST_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(usize);

impl RequestId {
    pub(crate) fn next() -> Self {
        RequestId(NEXT_REQUEST_ID.fetch_add(1, Ordering::SeqCst))
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct Upgrade<P> {
    pub(crate) protocols: SmallVec<[P; 2]>,
}

impl<P> Upgrade<P>
where
    P: AsRef<str> + Clone,
{
    pub fn new(protocols: SmallVec<[P; 2]>) -> Self {
        Self { protocols }
    }

    pub fn new_single(protocol: P) -> Self {
        Self {
            protocols: SmallVec::from_vec(vec![protocol]),
        }
    }
}

impl<P> UpgradeInfo for Upgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Info = P;
    type InfoIter = smallvec::IntoIter<[Self::Info; 2]>;

    fn protocol_info(&self) -> Self::InfoIter {
        self.protocols.clone().into_iter()
    }
}

impl<P> InboundUpgrade<Substream> for Upgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = (Substream, P);
    type Error = Infallible;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, io: Substream, protocol: Self::Info) -> Self::Future {
        future::ready(Ok((io, protocol)))
    }
}

impl<P> OutboundUpgrade<Substream> for Upgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = (Substream, P);
    type Error = Infallible;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, io: Substream, protocol: Self::Info) -> Self::Future {
        future::ready(Ok((io, protocol)))
    }
}

#[derive(Debug)]
pub struct Responder<TResponse> {
    tx: oneshot::Sender<TResponse>,
}

impl<TResponse> Responder<TResponse> {
    pub fn send_response(self, response: TResponse) -> Result<(), TResponse> {
        self.tx.send(response)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    request_timeout: Duration,
    // max_concurrent_streams: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(30),
            // max_concurrent_streams: 100,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OutboundFailure {
    #[error("Failed to dial the remote peer")]
    DialFailure,
    #[error("Timeout waiting for the response")]
    Timeout,
    #[error("Connection closed before response was received")]
    ConnectionClosed,
    #[error("Unsupported protocol for request")]
    UnsupportedProtocols,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum InboundFailure {
    #[error("Request timeout")]
    Timeout,
    #[error("Connection closed before response was received")]
    ConnectionClosed,
    #[error("Unsupported protocol for request")]
    UnsupportedProtocols,
    #[error("Response was dropped before it could be sent")]
    Discard,
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

impl From<InboundFailure> for io::Error {
    fn from(err: InboundFailure) -> Self {
        match err {
            InboundFailure::Timeout => io::Error::new(io::ErrorKind::TimedOut, err),
            InboundFailure::ConnectionClosed => io::Error::new(io::ErrorKind::UnexpectedEof, err),
            InboundFailure::UnsupportedProtocols => io::Error::new(io::ErrorKind::Other, err),
            InboundFailure::Discard => io::Error::new(io::ErrorKind::Other, err),
            InboundFailure::Io(e) => e,
        }
    }
}

impl From<OutboundFailure> for io::Error {
    fn from(err: OutboundFailure) -> Self {
        match err {
            OutboundFailure::DialFailure => io::Error::new(io::ErrorKind::ConnectionRefused, err),
            OutboundFailure::Timeout => io::Error::new(io::ErrorKind::TimedOut, err),
            OutboundFailure::ConnectionClosed => io::Error::new(io::ErrorKind::UnexpectedEof, err),
            OutboundFailure::UnsupportedProtocols => io::Error::new(io::ErrorKind::Other, err),
            OutboundFailure::Io(e) => e,
        }
    }
}
