use std::{
    convert::Infallible,
    io,
    task::{Context, Poll},
};

use futures::{
    StreamExt,
    channel::{mpsc, oneshot},
};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol,
};

use crate::{Upgrade, client::StreamError};

#[derive(Debug)]
pub(crate) struct NewStream {
    pub(crate) protocol: StreamProtocol,
    pub(crate) sender: oneshot::Sender<Result<Substream, StreamError>>,
}

pub struct Handler {
    receiver: mpsc::Receiver<NewStream>,
    /// 接收的新的出站流请求
    pending_outbound: Option<(
        StreamProtocol,
        oneshot::Sender<Result<Substream, StreamError>>,
    )>,
}

impl Handler {
    pub(crate) fn new(receiver: mpsc::Receiver<NewStream>) -> Self {
        Self {
            receiver,
            pending_outbound: None,
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

impl OutboundStreamHandler for Handler {
    type OutboundUpgrade = Upgrade;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        (stream, protocol): <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        let Some((expected_protocol, sender)) = self.pending_outbound.take() else {
            tracing::warn!(
                "Failed to establish outbound stream for protocol {:?}",
                protocol
            );
            return;
        };
        debug_assert!(protocol == expected_protocol);
        let _ = sender.send(Ok(stream));
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        let Some((protocol, sender)) = self.pending_outbound.take() else {
            tracing::warn!(
                "Failed to establish outbound stream for protocol {:?}",
                error
            );
            return;
        };

        let error = match error {
            StreamUpgradeError::Timeout => {
                StreamError::Io(io::Error::from(io::ErrorKind::TimedOut))
            }
            StreamUpgradeError::Apply(v) => unreachable!("Unexpected apply error: {:?}", v),
            StreamUpgradeError::NegotiationFailed => StreamError::Unsupported(protocol),
            StreamUpgradeError::Io(io) => StreamError::Io(io),
        };

        // 尝试发送错误到发送者
        let _ = sender.send(Err(error));
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if self.pending_outbound.is_some() {
            return Poll::Pending;
        }
        match self.receiver.poll_next_unpin(cx) {
            Poll::Ready(Some(NewStream {
                protocol, sender, ..
            })) => {
                self.pending_outbound = Some((protocol.clone(), sender));
                return Poll::Ready(SubstreamProtocol::new(
                    Upgrade {
                        supported_protocols: vec![protocol],
                    },
                    (),
                ));
            }
            Poll::Ready(None) => {}
            Poll::Pending => {}
        }
        Poll::Pending
    }
}
