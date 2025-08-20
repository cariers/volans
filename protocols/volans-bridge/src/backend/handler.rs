use std::{
    collections::VecDeque,
    convert::Infallible,
    fmt, io,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncWriteExt, FutureExt, SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
use volans_core::{Multiaddr, upgrade::ReadyUpgrade};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    StreamProtocol, Substream, SubstreamProtocol,
};

use crate::PROTOCOL_NAME;

/// 后端处理代理协议
pub struct Handler {
    circuit_addr: Multiaddr,
    pending_events: VecDeque<NewCircuitAccept>,
}

impl Handler {
    pub fn new(circuit_addr: Multiaddr) -> Self {
        Self {
            circuit_addr,
            pending_events: VecDeque::new(),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;
    type Event = NewCircuitAccept;

    fn handle_action(&mut self, _action: Self::Action) {
        // No actions to handle
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
    }

    fn poll_close(&mut self, _cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }
}

impl InboundStreamHandler for Handler {
    type InboundUpgrade = ReadyUpgrade<StreamProtocol>;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(ReadyUpgrade::new(PROTOCOL_NAME), ())
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        stream: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        // 有一个新流从Bridge连接到后端,
        // 将流发送给 Bridge Transport 处理, 将流升级为连接
        self.pending_events.push_back(NewCircuitAccept {
            stream,
            circuit_addr: self.circuit_addr.clone(),
        });
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
    }
}

pub struct NewCircuitAccept {
    pub(crate) circuit_addr: Multiaddr,
    pub(crate) stream: Substream,
}

impl fmt::Debug for NewCircuitAccept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NewCircuitAccept")
    }
}
