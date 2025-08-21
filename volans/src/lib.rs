pub use volans_core as core;

pub use core::Transport;

#[cfg(feature = "swarm")]
pub use volans_swarm as swarm;

#[cfg(feature = "codec")]
pub use volans_codec as codec;

// transports
#[cfg(feature = "tcp")]
pub use volans_tcp as tcp;

#[cfg(feature = "ws")]
pub use volans_ws as ws;

#[cfg(feature = "plaintext")]
pub use volans_plaintext as plaintext;

// multiplexing
#[cfg(feature = "muxing")]
pub use volans_muxing as muxing;

#[cfg(feature = "yamux")]
pub use volans_yamux as yamux;

// protocols
#[cfg(feature = "ping")]
pub use volans_ping as ping;

#[cfg(feature = "request")]
pub use volans_request as request;

#[cfg(feature = "stream")]
pub use volans_stream as stream;

#[cfg(feature = "registry")]
pub use volans_registry as registry;
