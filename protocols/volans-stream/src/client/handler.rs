use std::{
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    StreamExt,
    channel::{mpsc, oneshot},
};
use parking_lot::Mutex;
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, SubstreamProtocol,
};

use super::Shared;
use crate::OutboundStreamUpgradeFactory;

#[derive(Debug)]
pub(crate) struct NewOutboundStream<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    pub(crate) protocol: StreamProtocol,
    pub(crate) sender:
        oneshot::Sender<Result<TFactory::Output, StreamUpgradeError<TFactory::Error>>>,
}

pub struct Handler<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    receiver: mpsc::Receiver<NewOutboundStream<TFactory>>,
    shared: Arc<Mutex<Shared<TFactory>>>,
    /// 接收的新的出站流请求
    pending_outbound: Option<(
        StreamProtocol,
        oneshot::Sender<Result<TFactory::Output, StreamUpgradeError<TFactory::Error>>>,
    )>,
}

impl<TFactory> Handler<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    pub(crate) fn new(
        shared: Arc<Mutex<Shared<TFactory>>>,
        receiver: mpsc::Receiver<NewOutboundStream<TFactory>>,
    ) -> Self {
        Self {
            receiver,
            shared,
            pending_outbound: None,
        }
    }
}

impl<TFactory> ConnectionHandler for Handler<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    type Action = Infallible;
    type Event = Infallible;

    fn handle_action(&mut self, action: Self::Action) {
        unreachable!("Handler does not support actions, got: {:?}", action);
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
    }
}

impl<TFactory> OutboundStreamHandler for Handler<TFactory>
where
    TFactory: OutboundStreamUpgradeFactory,
{
    type OutboundUpgrade = TFactory::Upgrade;
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
        let Some((_protocol, sender)) = self.pending_outbound.take() else {
            tracing::warn!(
                "Failed to establish outbound stream for protocol {:?}",
                error
            );
            return;
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
            Poll::Ready(Some(NewOutboundStream {
                protocol, sender, ..
            })) => {
                self.pending_outbound = Some((protocol.clone(), sender));
                let upgrade = Shared::lock(&self.shared).outbound_request(protocol);
                return Poll::Ready(upgrade);
            }
            Poll::Ready(None) => {}
            Poll::Pending => {}
        }
        Poll::Pending
    }
}
