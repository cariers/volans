use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::Arc,
    task::{Context, Poll},
};

use parking_lot::Mutex;
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    SubstreamProtocol,
};

use crate::{InboundStreamUpgradeFactory, StreamEvent, server::Shared};

pub struct Handler<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    shared: Arc<Mutex<Shared<TFactory>>>,
    pending_events: VecDeque<StreamEvent<TFactory::Output, TFactory::Error>>,
}

impl<TFactory> Handler<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    pub(crate) fn new(shared: Arc<Mutex<Shared<TFactory>>>) -> Self {
        Self {
            shared,
            pending_events: VecDeque::new(),
        }
    }
}

impl<TFactory> ConnectionHandler for Handler<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    type Action = Infallible;
    type Event = StreamEvent<TFactory::Output, TFactory::Error>;

    fn handle_action(&mut self, action: Self::Action) {
        unreachable!("Handler does not support actions, got: {:?}", action);
    }

    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        if let Some(event) = self.pending_events.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::Notify(event));
        }
        Poll::Pending
    }
}

impl<TFactory> InboundStreamHandler for Handler<TFactory>
where
    TFactory: InboundStreamUpgradeFactory,
{
    type InboundUpgrade = TFactory::Upgrade;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        Shared::lock(&self.shared).listen_protocol()
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        (output, protocol): <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        self.pending_events
            .push_back(StreamEvent::FullyNegotiated { output, protocol });
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        self.pending_events
            .push_back(StreamEvent::UpgradeError { error });
    }
}
