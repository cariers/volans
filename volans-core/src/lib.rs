mod connection;
mod extensions;

pub mod either;

pub mod identity;
pub mod multiaddr;
pub mod muxing;
pub mod transport;
pub mod upgrade;

pub use connection::{ConnectedPoint, Endpoint};
pub use extensions::Extensions;
pub use identity::PeerId;
pub use multiaddr::Multiaddr;
pub use muxing::StreamMuxer;
pub use transport::{Listener, ListenerEvent, Transport, TransportError};
pub use upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};

pub use ed25519_dalek;

pub type Negotiated<T> = volans_stream_select::Negotiated<T>;
