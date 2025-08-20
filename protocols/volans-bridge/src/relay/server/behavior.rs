use std::{
    collections::VecDeque,
    convert::Infallible,
    task::{Context, Poll},
};

use futures::channel::mpsc;
use volans_core::{Multiaddr, PeerId};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, NetworkBehavior, NetworkIncomingBehavior,
    THandlerAction, THandlerEvent,
};

use crate::relay::CircuitRequest;

use super::handler;

pub struct Behavior {
    pending_requests: VecDeque<CircuitRequest>,
    request_sender: mpsc::UnboundedSender<CircuitRequest>,
}

impl Behavior {
    pub fn new(request_sender: mpsc::UnboundedSender<CircuitRequest>) -> Self {
        Self {
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
        handler::CircuitRequest {
            dst_peer_id,
            stream,
        }: THandlerEvent<Self>,
    ) {
        // 客户端发起一个中继请求,
        let request = CircuitRequest {
            dst_peer_id,
            src_peer_id: peer_id,
            src_connection_id: id,
            src_stream: stream,
        };
        // 写入待处理请求队列
        self.pending_requests.push_back(request);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        loop {
            if let Some(request) = self.pending_requests.pop_front() {
                // 发送请求给客户端
                tracing::error!("Sending request: {:?}", request);
                if let Err(err) = self.request_sender.unbounded_send(request) {
                    tracing::error!("Failed to send request: {:?}", err);
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
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(handler::Handler::new())
    }
}
