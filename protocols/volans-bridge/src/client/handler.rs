use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    task::{Context, Poll},
    time::Duration,
};

use futures::{FutureExt, channel::oneshot};
use futures_bounded::{Delay, FuturesTupleSet};
use volans_codec::Bytes;
use volans_core::{Multiaddr, PeerId, upgrade::ReadyUpgrade};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol,
};

use crate::{protocol, transport::Connection};

pub struct Handler {
    outbound_requests: VecDeque<NewOutboundBridgeRequest>,
    pending_outbound: Option<NewOutboundBridgeRequest>,
    outbound_circuit_requests: FuturesTupleSet<
        Result<(Substream, Bytes), protocol::ConnectError>,
        oneshot::Sender<Result<Connection, protocol::ConnectError>>,
    >,
}

impl Handler {
    pub fn new(timeout: Duration) -> Self {
        Self {
            outbound_requests: VecDeque::new(),
            pending_outbound: None,
            outbound_circuit_requests: FuturesTupleSet::new(
                move || Delay::futures_timer(timeout),
                10,
            ),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = NewOutboundBridgeRequest;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        // 等待处理的请求
        self.outbound_requests.push_back(action);
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            match self.outbound_circuit_requests.poll_unpin(cx) {
                Poll::Ready((Ok(Ok((stream, read_buffer))), send_back)) => {
                    tracing::debug!("Outbound circuit request succeeded");
                    let _ = send_back.send(Ok(Connection::new_accepted(stream, read_buffer)));
                    continue;
                }
                Poll::Ready((Ok(Err(error)), send_back)) => {
                    tracing::debug!("Outbound circuit request failed: {}", error);
                    let _ = send_back.send(Err(error));
                    continue;
                }
                Poll::Ready((Err(error), _)) => {
                    tracing::debug!("Outbound circuit request failed: {}", error);
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

impl OutboundStreamHandler for Handler {
    type OutboundUpgrade = ReadyUpgrade<StreamProtocol>;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        stream: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        let NewOutboundBridgeRequest {
            dst_peer_id,
            send_back,
            ..
        } = self
            .pending_outbound
            .take()
            .expect("Pending request should exist");

        tracing::debug!(
            "Bridge upgrade successful for backend peer: {:?}",
            dst_peer_id
        );

        let result = self.outbound_circuit_requests.try_push(
            protocol::make_bridge_connect(stream, dst_peer_id, vec![]).boxed(),
            send_back,
        );

        if result.is_err() {
            tracing::warn!("Drop pending outbound request: because we are at capacity");
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        // 升级失败，通知请求者
        let NewOutboundBridgeRequest {
            dst_peer_id,
            send_back,
            ..
        } = self
            .pending_outbound
            .take()
            .expect("Pending request should exist");

        let error = match error {
            StreamUpgradeError::Timeout => protocol::ConnectError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "Outbound upgrade timed out",
            )),
            StreamUpgradeError::NegotiationFailed => protocol::ConnectError::Unsupported,
            StreamUpgradeError::Io(err) => protocol::ConnectError::Io(err),
            StreamUpgradeError::Apply(_) => unreachable!("Apply error should not happen here"),
        };
        tracing::debug!("Bridge upgrade error for peer {:?}: {}", dst_peer_id, error);
        let _ = send_back.send(Err(error));
    }

    fn poll_outbound_request(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if self.pending_outbound.is_none() {
            if let Some(request) = self.outbound_requests.pop_front() {
                tracing::debug!(
                    "Preparing outbound request to relay: {:?}",
                    request.dst_peer_id
                );
                let upgrade = ReadyUpgrade::new(protocol::PROTOCOL_NAME);
                self.pending_outbound = Some(request);
                // 准备发送请求
                return Poll::Ready(SubstreamProtocol::new(upgrade, ()));
            }
        }
        Poll::Pending
    }
}

pub struct NewOutboundBridgeRequest {
    pub(crate) dst_addresses: Vec<Multiaddr>,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) send_back: oneshot::Sender<Result<Connection, protocol::ConnectError>>,
}

impl fmt::Debug for NewOutboundBridgeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewOutboundBridgeRequest")
            .field("dst_peer_id", &self.dst_peer_id)
            .finish()
    }
}
