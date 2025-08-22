use std::{
    convert::Infallible,
    fmt,
    task::{Context, Poll},
    time::Duration,
};

use futures::FutureExt;
use futures_bounded::FuturesSet;
use volans_core::{Multiaddr, PeerId, upgrade::ReadyUpgrade};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    StreamProtocol, SubstreamProtocol,
};

use crate::protocol;

/// 后端处理代理协议
pub struct Handler {
    relay_remote_addr: Multiaddr,
    inbound_pending_circuits: FuturesSet<Result<protocol::Relay, protocol::Error>>,
}

impl Handler {
    pub fn new(relay_remote_addr: Multiaddr) -> Self {
        Self {
            relay_remote_addr,
            inbound_pending_circuits: FuturesSet::new(
                || futures_bounded::Delay::futures_timer(Duration::from_secs(5)),
                10, // 最大并行处理数
            ),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;
    type Event = NewCircuitAccept;

    fn handle_action(&mut self, _action: Self::Action) {
        // No actions to handle
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            match self.inbound_pending_circuits.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(protocol::Relay {
                    circuit,
                    src_peer_id,
                    dst_peer_id,
                    src_relayed_addr,
                }))) => {
                    tracing::info!("Inbound circuit request accepted");
                    let event = NewCircuitAccept {
                        relay_remote_addr: self.relay_remote_addr.clone(),
                        circuit,
                        src_peer_id,
                        dst_peer_id,
                        src_relayed_addr,
                    };
                    return Poll::Ready(ConnectionHandlerEvent::Notify(event));
                }
                Poll::Ready(Ok(Err(e))) => {
                    tracing::warn!("Inbound circuit error: {:?}", e);
                    continue;
                }
                Poll::Ready(Err(e)) => {
                    tracing::error!("Inbound circuit processing failed: {:?}", e);
                    continue;
                }
                Poll::Pending => {}
            }
            return Poll::Pending;
        }
    }

    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }
}

impl InboundStreamHandler for Handler {
    type InboundUpgrade = ReadyUpgrade<StreamProtocol>;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ())
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        stream: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        let result = self
            .inbound_pending_circuits
            .try_push(protocol::handle_bridge_relay_connect(stream).boxed());
        if result.is_err() {
            tracing::warn!("Failed to push inbound circuit");
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        tracing::warn!("New circuit accept eRR");
    }
}

pub struct NewCircuitAccept {
    pub(crate) relay_remote_addr: Multiaddr,
    pub(crate) circuit: protocol::Circuit,
    pub(crate) src_peer_id: PeerId,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) src_relayed_addr: Multiaddr,
}

impl fmt::Debug for NewCircuitAccept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewCircuitAccept")
            .field("src_peer_id", &self.src_peer_id)
            .field("dst_peer_id", &self.dst_peer_id)
            .field("src_relayed_addr", &self.src_relayed_addr)
            .finish()
    }
}
