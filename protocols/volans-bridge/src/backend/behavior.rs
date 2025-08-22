use std::{
    collections::HashMap,
    convert::Infallible,
    task::{Context, Poll},
};

use futures::{StreamExt, channel::mpsc};
use volans_core::{Multiaddr, PeerId, multiaddr::Protocol};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, NetworkBehavior, NetworkIncomingBehavior,
    THandlerAction, THandlerEvent,
};

use crate::transport::{Connection, IncomingRelayedConnection, TransportRequest};

use super::handler;

pub struct Behavior {
    transport_request_receiver: mpsc::Receiver<TransportRequest>,
    listener: Option<mpsc::Sender<IncomingRelayedConnection>>,
}

impl Behavior {
    pub fn new(transport_request_receiver: mpsc::Receiver<TransportRequest>) -> Self {
        Self {
            transport_request_receiver,
            listener: None,
        }
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = handler::Handler;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        _id: ConnectionId,
        peer_id: PeerId,
        handler::NewCircuitAccept {
            relay_remote_addr,
            circuit,
            src_peer_id,
            dst_peer_id: _,
            src_relayed_addr,
        }: THandlerEvent<Self>,
    ) {
        match self.listener {
            Some(ref mut sender) => {
                let r = sender.try_send(IncomingRelayedConnection::new(
                    Connection::new_accepting(circuit),
                    src_peer_id,
                    peer_id,
                    src_relayed_addr,
                ));
                if let Err(e) = r {
                    tracing::error!("Failed to send incoming relayed connection: {}", e);
                }
            }
            None => {
                tracing::warn!(
                    "No listener found for remote address: {}",
                    relay_remote_addr
                );
            }
        }
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        loop {
            match self.transport_request_receiver.poll_next_unpin(cx) {
                Poll::Ready(Some(TransportRequest::ListenRequest {
                    local_addr,
                    listener_sender,
                })) => {
                    tracing::debug!("Circuit Listening on: {:?}", local_addr);
                    self.listener = Some(listener_sender);
                    continue;
                }
                Poll::Ready(Some(TransportRequest::DialRequest { .. })) => {
                    tracing::error!("Unexpected DialRequest in Backend Behavior");
                }
                Poll::Ready(None) => {}
                Poll::Pending => {}
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
        _local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        let relay_addr = remote_addr.clone().with(Protocol::Peer(peer_id));
        Ok(handler::Handler::new(relay_addr))
    }
}
