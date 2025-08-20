use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncReadExt, AsyncWriteExt, FutureExt, SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
use volans_core::{InboundUpgrade, PeerId, UpgradeInfo};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    StreamProtocol, Substream, SubstreamProtocol,
};

use crate::PROTOCOL_NAME;

/// 中继服务器处理前端客户端求，
/// 通过 relay client 连接到后端 backend
pub struct Handler {
    pending_events: VecDeque<CircuitRequest>,
}

impl Handler {
    pub fn new() -> Self {
        Self {
            pending_events: VecDeque::new(),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;
    type Event = CircuitRequest;

    fn handle_action(&mut self, _action: Self::Action) {
        // No actions to handle
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(ConnectionHandlerEvent::Notify(event));
            }
            return Poll::Pending;
        }
    }

    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }
}

impl InboundStreamHandler for Handler {
    type InboundUpgrade = DestinationUpgrade;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(DestinationUpgrade, ())
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        (dst_peer_id, stream): <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        // 有一个新流从客户端连接到Relay Server
        // 将流发送给 Bridge Transport 处理, 将流升级为连接
        let request = CircuitRequest {
            dst_peer_id,
            stream,
        };
        self.pending_events.push_back(request);
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        // Handle upgrade errors
    }
}

pub struct DestinationUpgrade;

impl UpgradeInfo for DestinationUpgrade {
    type Info = StreamProtocol;
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(PROTOCOL_NAME)
    }
}

impl InboundUpgrade<Substream> for DestinationUpgrade {
    type Output = (PeerId, Substream);
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;

    fn upgrade_inbound(self, mut socket: Substream, _info: Self::Info) -> Self::Future {
        async move {
            let mut peer_buffer = [0u8; 32];
            socket.read_exact(&mut peer_buffer).await?;
            let peer_id = PeerId::from_bytes(peer_buffer);
            Ok((peer_id, socket))
        }
        .boxed()
    }
}

/// 回路请求
pub struct CircuitRequest {
    pub(crate) dst_peer_id: PeerId,
    pub(crate) stream: Substream,
}

impl fmt::Debug for CircuitRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitRequest")
            .field("dst_peer_id", &self.dst_peer_id)
            .finish()
    }
}
