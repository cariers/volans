use std::{
    convert::Infallible,
    task::{Context, Poll},
};

use volans_core::upgrade::{DeniedUpgrade, PendingUpgrade};

use crate::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    OutboundStreamHandler, StreamUpgradeError, SubstreamProtocol,
};

#[derive(Clone, Debug)]
pub struct DummyHandler;

impl ConnectionHandler for DummyHandler {
    type Action = Infallible;
    type Event = Infallible;
    fn handle_action(&mut self, _action: Self::Action) {
        unreachable!("PendingConnectionHandler does not handle actions");
    }
    fn poll(&mut self, _cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        Poll::Pending
    }
}

impl InboundStreamHandler for DummyHandler {
    type InboundUpgrade = DeniedUpgrade;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(DeniedUpgrade, ())
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        _protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        unreachable!("PendingConnectionHandler does not support fully negotiated protocols",);
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        unreachable!("PendingConnectionHandler does not support upgrade errors");
    }
}

impl OutboundStreamHandler for DummyHandler {
    type OutboundUpgrade = PendingUpgrade<String>;
    type OutboundUserData = Infallible;

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        _protocol: <Self::OutboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        unreachable!("PendingConnectionHandler does not support fully negotiated protocols");
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        _error: StreamUpgradeError<<Self::OutboundUpgrade as InboundUpgradeSend>::Error>,
    ) {
        unreachable!("PendingConnectionHandler does not support upgrade errors");
    }

    fn poll_outbound_request(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        Poll::Pending
    }
}
