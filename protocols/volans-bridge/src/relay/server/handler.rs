use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt,
    task::{Context, Poll},
    time::Duration,
};

use futures::FutureExt;
use futures_bounded::{Delay, FuturesSet};
use volans_core::{Multiaddr, PeerId, upgrade::ReadyUpgrade};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    StreamProtocol, SubstreamProtocol,
};

use crate::protocol;

/// 中继服务器处理前端客户端求，
/// 通过 relay client 连接到后端 backend
pub struct Handler {
    pending_events: VecDeque<CircuitAccepted>,
    inbound_circuit_requests: FuturesSet<Result<protocol::Bridge, protocol::Error>>,
    relayed_addr: Multiaddr,
}

impl Handler {
    pub fn new(relayed_addr: Multiaddr) -> Self {
        Self {
            relayed_addr,
            pending_events: VecDeque::new(),
            inbound_circuit_requests: FuturesSet::new(
                || Delay::futures_timer(Duration::from_secs(15)),
                10, // 最大同时处理
            ),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;
    type Event = CircuitAccepted;

    fn handle_action(&mut self, _action: Self::Action) {
        // No actions to handle
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(ConnectionHandlerEvent::Notify(event));
            }
            match self.inbound_circuit_requests.poll_unpin(cx) {
                Poll::Ready(Ok(Ok(protocol::Bridge {
                    circuit,
                    dst_peer_id,
                    dst_addresses,
                }))) => {
                    let event = CircuitAccepted {
                        relayed_addr: self.relayed_addr.clone(),
                        circuit,
                        dst_peer_id,
                        dst_addresses,
                    };
                    return Poll::Ready(ConnectionHandlerEvent::Notify(event));
                }
                Poll::Ready(Ok(Err(err))) => {
                    tracing::error!("Inbound circuit request failed: {:?}", err);
                    continue;
                }
                Poll::Ready(Err(_)) => {
                    tracing::error!("Inbound circuit request timeout");
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
            .inbound_circuit_requests
            .try_push(protocol::handle_bridge_connect(stream).boxed());

        if result.is_err() {
            tracing::warn!("Failed to push inbound circuit request(channel full), dropping stream");
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
    }
}

pub struct CircuitAccepted {
    pub(crate) relayed_addr: Multiaddr,
    pub(crate) circuit: protocol::Circuit,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) dst_addresses: Vec<Multiaddr>,
}
impl fmt::Debug for CircuitAccepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitAccepted")
            .field("relayed_addr", &self.relayed_addr)
            .field("dst_peer_id", &self.dst_peer_id)
            .field("dst_addresses", &self.dst_addresses)
            .finish()
    }
}
