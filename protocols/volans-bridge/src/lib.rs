use volans_core::{Multiaddr, multiaddr::Protocol};
use volans_swarm::StreamProtocol;

// 后端处理
pub mod backend;
// 客户端处理
pub mod client;
// 中继服务，包括客户端和服务端
pub mod relay;

pub(crate) mod protocol;
pub mod transport;

pub(crate) trait MultiaddrExt {
    fn is_circuit(&self) -> bool;
}

impl MultiaddrExt for Multiaddr {
    fn is_circuit(&self) -> bool {
        self.iter().any(|p| p == Protocol::Circuit)
    }
}
