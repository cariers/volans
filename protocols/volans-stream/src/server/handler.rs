use std::{
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use parking_lot::Mutex;
use volans_core::PeerId;
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, ConnectionId, InboundStreamHandler,
    InboundUpgradeSend, SubstreamProtocol,
};

use crate::Upgrade;

use super::shared::Shared;

pub struct Handler {
    remote_peer_id: PeerId,
    connection_id: ConnectionId,
    shared: Arc<Mutex<Shared>>,
}

impl Handler {
    pub(crate) fn new(
        remote_peer_id: PeerId,
        connection_id: ConnectionId,
        shared: Arc<Mutex<Shared>>,
    ) -> Self {
        Self {
            remote_peer_id,
            connection_id,
            shared,
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        unreachable!("Handler does not support actions, got: {:?}", action);
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
    }
}

impl InboundStreamHandler for Handler {
    type InboundUpgrade = Upgrade;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(
            Upgrade {
                supported_protocols: Shared::lock(&self.shared).supported_protocols(),
            },
            (),
        )
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        (stream, protocol): <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        Shared::lock(&self.shared).on_inbound_stream(
            self.remote_peer_id,
            self.connection_id,
            stream,
            protocol,
        );
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
    }
}
