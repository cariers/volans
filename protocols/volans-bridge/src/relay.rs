use std::fmt;

use futures::channel::mpsc;
use volans_core::{Multiaddr, PeerId};
use volans_swarm::ConnectionId;

use crate::protocol;

pub mod client;

pub mod server;

pub fn new(local_peer_id: PeerId) -> (server::Behavior, client::Behavior) {
    let (tx, rx) = mpsc::unbounded();

    let server_behavior = server::Behavior::new(local_peer_id, tx);
    let client_behavior = client::Behavior::new(rx);
    (server_behavior, client_behavior)
}

/// 中继服务端与中继客户端之间的请求
pub struct CircuitRequest {
    pub relayed_addr: Multiaddr,
    pub dst_peer_id: PeerId,
    pub dst_addresses: Vec<Multiaddr>,
    pub circuit: protocol::Circuit,
    pub src_peer_id: PeerId,
    pub src_connection_id: ConnectionId,
}

impl fmt::Debug for CircuitRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitRequest")
            .field("dst_peer_id", &self.dst_peer_id)
            .field("src_peer_id", &self.src_peer_id)
            .field("src_connection_id", &self.src_connection_id)
            .field("relayed_addr", &self.relayed_addr)
            .finish()
    }
}
