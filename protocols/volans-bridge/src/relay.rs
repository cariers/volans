use std::fmt;

use futures::channel::mpsc;
use volans_core::PeerId;
use volans_swarm::{ConnectionId, Substream};

pub mod client;

pub mod server;

pub fn new() -> (server::Behavior, client::Behavior) {
    let (tx, rx) = mpsc::unbounded();

    let server_behavior = server::Behavior::new(tx);
    let client_behavior = client::Behavior::new(rx);
    (server_behavior, client_behavior)
}

/// 中继服务端与中继客户端之间的请求
pub struct CircuitRequest {
    pub dst_peer_id: PeerId,
    pub src_stream: Substream,
    pub src_peer_id: PeerId,
    pub src_connection_id: ConnectionId,
}

impl fmt::Debug for CircuitRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitRequest")
            .field("dst_peer_id", &self.dst_peer_id)
            .field("src_peer_id", &self.src_peer_id)
            .field("src_connection_id", &self.src_connection_id)
            .finish()
    }
}
