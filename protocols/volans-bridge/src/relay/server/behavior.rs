use std::{
    collections::VecDeque,
    convert::Infallible,
    task::{Context, Poll},
};

use futures::channel::mpsc;
use volans_core::{Multiaddr, PeerId, multiaddr::Protocol};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, NetworkBehavior, NetworkIncomingBehavior,
    THandlerAction, THandlerEvent,
};

use crate::relay::CircuitRequest;

use super::handler;

pub struct Behavior {
    local_peer_id: PeerId,
    pending_requests: VecDeque<CircuitRequest>,
    request_sender: mpsc::UnboundedSender<CircuitRequest>,
}

impl Behavior {
    pub fn new(
        local_peer_id: PeerId,
        request_sender: mpsc::UnboundedSender<CircuitRequest>,
    ) -> Self {
        Self {
            local_peer_id,
            pending_requests: VecDeque::new(),
            request_sender,
        }
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = handler::Handler;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        handler::CircuitAccepted {
            dst_peer_id,
            dst_addresses,
            circuit,
            relayed_addr,
        }: THandlerEvent<Self>,
    ) {
        // 客户端发起一个中继请求,
        let request = CircuitRequest {
            relayed_addr,
            dst_peer_id,
            dst_addresses,
            src_peer_id: peer_id,
            src_connection_id: id,
            circuit,
        };
        // 写入待处理请求队列
        self.pending_requests.push_back(request);
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        loop {
            if let Some(request) = self.pending_requests.pop_front() {
                // 发送请求给客户端
                tracing::debug!("Sending request: {:?}", request);
                if let Err(err) = self.request_sender.unbounded_send(request) {
                    tracing::warn!("Failed to send request: {:?}", err);
                }
                continue;
            }
            return Poll::Pending;
        }
    }
}

impl NetworkIncomingBehavior for Behavior {
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        peer_id: PeerId,
        local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        let relay_addr = local_addr
            .clone()
            .with(Protocol::Peer(self.local_peer_id))
            .with(Protocol::Circuit)
            .with(Protocol::Peer(peer_id));

        Ok(handler::Handler::new(relay_addr))
    }
}
