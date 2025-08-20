use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncWriteExt, FutureExt, channel::oneshot};
use volans_core::{Multiaddr, OutboundUpgrade, PeerId, UpgradeInfo};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol,
};

use crate::PROTOCOL_NAME;

pub struct Handler {
    outbound_requests: VecDeque<NewOutboundBridgeRequest>,
    pending_outbound: Option<NewOutboundBridgeRequest>,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            outbound_requests: VecDeque::new(),
            pending_outbound: None,
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = NewOutboundBridgeRequest;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        // 等待处理的请求

        tracing::error!(
            "Received outbound request to relay: {:?} -> {:?}",
            action.relay_peer_id,
            action.dst_peer_id
        );

        self.outbound_requests.push_back(action);
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
    }

    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }
}

impl OutboundStreamHandler for Handler {
    type OutboundUpgrade = DestinationUpgrade;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        stream: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        // 升级失败，通知请求者
        let NewOutboundBridgeRequest {
            relay_addr: _,
            relay_peer_id: _,
            dst_peer_id: _,
            send_back,
        } = self
            .pending_outbound
            .take()
            .expect("Pending request should exist");

        let _ = send_back.send(Ok(stream));
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        _error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        // 升级失败，通知请求者
        let NewOutboundBridgeRequest {
            relay_addr,
            relay_peer_id: _,
            dst_peer_id,
            send_back,
        } = self
            .pending_outbound
            .take()
            .expect("Pending request should exist");

        let _ = send_back.send(Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "Failed to upgrade outbound stream to {}: {}",
                relay_addr, dst_peer_id
            ),
        )));
    }

    fn poll_outbound_request(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if self.pending_outbound.is_none() {
            if let Some(request) = self.outbound_requests.pop_front() {
                tracing::error!(
                    "Preparing outbound request to relay: {:?} -> {:?}",
                    request.relay_peer_id,
                    request.dst_peer_id
                );
                let upgrade = DestinationUpgrade::new(request.dst_peer_id.clone());
                self.pending_outbound = Some(request);
                // 准备发送请求
                return Poll::Ready(SubstreamProtocol::new(upgrade, ()));
            }
        }
        Poll::Pending
    }
}

pub struct NewOutboundBridgeRequest {
    pub(crate) relay_addr: Multiaddr,
    pub(crate) relay_peer_id: PeerId,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) send_back: oneshot::Sender<Result<Substream, io::Error>>,
}

impl fmt::Debug for NewOutboundBridgeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewOutboundBridgeRequest")
            .field("relay_addr", &self.relay_addr)
            .field("relay_peer_id", &self.relay_peer_id)
            .field("dst_peer_id", &self.dst_peer_id)
            .finish()
    }
}

pub struct DestinationUpgrade {
    dst_peer_id: PeerId,
}

impl DestinationUpgrade {
    pub fn new(dst_peer_id: PeerId) -> Self {
        Self { dst_peer_id }
    }
}

impl UpgradeInfo for DestinationUpgrade {
    type Info = StreamProtocol;
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(PROTOCOL_NAME)
    }
}

impl OutboundUpgrade<Substream> for DestinationUpgrade {
    type Output = Substream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

    fn upgrade_outbound(self, mut socket: Substream, _info: Self::Info) -> Self::Future {
        async move {
            let peer_buffer = self.dst_peer_id.clone().into_bytes();
            socket.write_all(&peer_buffer).await?;
            Ok(socket)
        }
        .boxed()
    }
}
