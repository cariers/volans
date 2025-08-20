use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use futures::channel::mpsc;
use volans_core::{Multiaddr, PeerId, multiaddr::Protocol};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, Substream, THandlerAction, THandlerEvent,
    error::{ConnectionError, ListenError},
};

use crate::transport::TransportRequest;

use super::handler;

pub struct Behavior {
    transport_request_receiver: mpsc::Receiver<TransportRequest>,
}

impl Behavior {
    pub fn new(transport_request_receiver: mpsc::Receiver<TransportRequest>) -> Self {
        Self {
            transport_request_receiver,
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
        handler::NewCircuitAccept {
            stream,
            circuit_addr: remote_addr,
        }: THandlerEvent<Self>,
    ) {
        todo!()
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        Poll::Pending
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
        let circuit_addr = remote_addr.clone().with(Protocol::Peer(peer_id));
        Ok(handler::Handler::new(circuit_addr.clone()))
    }
}
