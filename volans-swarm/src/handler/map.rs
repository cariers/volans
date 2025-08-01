use std::{
    fmt,
    marker::PhantomData,
    task::{Context, Poll},
};

use crate::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    OutboundStreamHandler, OutboundUpgradeSend, StreamUpgradeError, SubstreamProtocol,
};

#[derive(Debug)]
pub struct MapEvent<THandler, TMap> {
    inner: THandler,
    map: TMap,
}

impl<THandler, TMap> MapEvent<THandler, TMap> {
    pub(crate) fn new(inner: THandler, map: TMap) -> Self {
        Self { inner, map }
    }
}

impl<THandler, O, TMap> ConnectionHandler for MapEvent<THandler, TMap>
where
    THandler: ConnectionHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: FnMut(THandler::Event) -> O,
{
    type Action = THandler::Action;
    type Event = O;

    fn handle_action(&mut self, action: Self::Action) {
        self.inner.handle_action(action);
    }

    fn connection_keep_alive(&self) -> bool {
        self.inner.connection_keep_alive()
    }

    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        self.inner.poll_close(cx).map(|e| e.map(|e| (self.map)(e)))
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        self.inner.poll(cx).map(|e| e.map_event(|e| (self.map)(e)))
    }
}

impl<THandler, O, TMap> InboundStreamHandler for MapEvent<THandler, TMap>
where
    THandler: InboundStreamHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: FnMut(THandler::Event) -> O,
{
    type InboundUpgrade = THandler::InboundUpgrade;
    type InboundUserData = THandler::InboundUserData;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        self.inner.listen_protocol()
    }

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        self.inner.on_fully_negotiated(user_data, protocol);
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        self.inner.on_upgrade_error(user_data, error);
    }
}

impl<THandler, O, TMap> OutboundStreamHandler for MapEvent<THandler, TMap>
where
    THandler: OutboundStreamHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: FnMut(THandler::Event) -> O,
{
    type OutboundUpgrade = THandler::OutboundUpgrade;
    type OutboundUserData = THandler::OutboundUserData;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::OutboundUserData,
        protocol: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        self.inner.on_fully_negotiated(user_data, protocol);
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        self.inner.on_upgrade_error(user_data, error);
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        self.inner.poll_outbound_request(cx)
    }
}

#[derive(Debug)]
pub struct MapAction<THandler, O, TMap> {
    inner: THandler,
    map: TMap,
    _marker: PhantomData<O>,
}

impl<THandler, O, TMap> MapAction<THandler, O, TMap> {
    pub(crate) fn new(inner: THandler, map: TMap) -> Self {
        Self {
            inner,
            map,
            _marker: PhantomData,
        }
    }
}

impl<THandler, O, TMap> ConnectionHandler for MapAction<THandler, O, TMap>
where
    THandler: ConnectionHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: Fn(O) -> Option<THandler::Action>,
{
    type Action = O;
    type Event = THandler::Event;

    fn handle_action(&mut self, action: Self::Action) {
        if let Some(action) = (self.map)(action) {
            self.inner.handle_action(action);
        }
    }

    fn connection_keep_alive(&self) -> bool {
        self.inner.connection_keep_alive()
    }

    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        self.inner.poll_close(cx)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        self.inner.poll(cx)
    }
}

impl<THandler, O, TMap> InboundStreamHandler for MapAction<THandler, O, TMap>
where
    THandler: InboundStreamHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: Fn(O) -> Option<THandler::Action>,
{
    type InboundUpgrade = THandler::InboundUpgrade;
    type InboundUserData = THandler::InboundUserData;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        self.inner.listen_protocol()
    }

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        self.inner.on_fully_negotiated(user_data, protocol);
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        self.inner.on_upgrade_error(user_data, error);
    }
}

impl<THandler, O, TMap> OutboundStreamHandler for MapAction<THandler, O, TMap>
where
    THandler: OutboundStreamHandler,
    O: fmt::Debug + Send + Clone + 'static,
    TMap: Send + 'static,
    TMap: Fn(O) -> Option<THandler::Action>,
{
    type OutboundUpgrade = THandler::OutboundUpgrade;
    type OutboundUserData = THandler::OutboundUserData;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::OutboundUserData,
        protocol: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        self.inner.on_fully_negotiated(user_data, protocol);
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        self.inner.on_upgrade_error(user_data, error);
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        self.inner.poll_outbound_request(cx)
    }
}
