use std::{
    cmp,
    task::{Context, Poll},
};

use either::Either;
use futures::{future, ready};
use volans_core::upgrade::SelectUpgrade;

use crate::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    OutboundStreamHandler, OutboundUpgradeSend, StreamUpgradeError, SubstreamProtocol,
    upgrade::SendWrapper,
};

#[derive(Debug, Clone)]
pub struct ConnectionHandlerSelect<THandler1, THandler2> {
    first: THandler1,
    second: THandler2,
}

impl<THandler1, THandler2> ConnectionHandlerSelect<THandler1, THandler2> {
    pub fn select(first: THandler1, second: THandler2) -> Self {
        Self { first, second }
    }
}

impl<THandler1, THandler2> ConnectionHandler for ConnectionHandlerSelect<THandler1, THandler2>
where
    THandler1: ConnectionHandler,
    THandler2: ConnectionHandler,
{
    type Action = Either<THandler1::Action, THandler2::Action>;
    type Event = Either<THandler1::Event, THandler2::Event>;

    fn handle_action(&mut self, action: Self::Action) {
        match action {
            Either::Left(action) => self.first.handle_action(action),
            Either::Right(action) => self.second.handle_action(action),
        }
    }

    fn connection_keep_alive(&self) -> bool {
        self.first.connection_keep_alive() || self.second.connection_keep_alive()
    }

    fn poll_close(&mut self, cx: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        if let Some(e) = ready!(self.first.poll_close(cx)) {
            return Poll::Ready(Some(Either::Left(e)));
        }

        if let Some(e) = ready!(self.second.poll_close(cx)) {
            return Poll::Ready(Some(Either::Right(e)));
        }

        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        match self.first.poll(cx) {
            Poll::Ready(event) => return Poll::Ready(event.map_event(Either::Left)),
            Poll::Pending => {}
        };
        match self.second.poll(cx) {
            Poll::Ready(event) => return Poll::Ready(event.map_event(Either::Right)),
            Poll::Pending => {}
        };
        Poll::Pending
    }
}

impl<THandler1, THandler2> InboundStreamHandler for ConnectionHandlerSelect<THandler1, THandler2>
where
    THandler1: InboundStreamHandler,
    THandler2: InboundStreamHandler,
{
    type InboundUpgrade = SelectUpgrade<
        SendWrapper<THandler1::InboundUpgrade>,
        SendWrapper<THandler2::InboundUpgrade>,
    >;
    type InboundUserData = (THandler1::InboundUserData, THandler2::InboundUserData);

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        let first = self.first.listen_protocol();
        let second = self.second.listen_protocol();
        let (upgrade1, info1, timeout1) = first.into_inner();
        let (upgrade2, info2, timeout2) = second.into_inner();
        let timeout = cmp::max(timeout1, timeout2);
        let choice = SelectUpgrade::new(SendWrapper(upgrade1), SendWrapper(upgrade2));
        SubstreamProtocol::new(choice, (info1, info2)).with_timeout(timeout)
    }

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        match protocol {
            future::Either::Left(output) => {
                self.first.on_fully_negotiated(user_data.0, output);
            }
            future::Either::Right(output) => {
                self.second.on_fully_negotiated(user_data.1, output);
            }
        }
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        match error {
            Either::Left(err) => {
                self.first.on_upgrade_error(user_data.0, err);
            }
            Either::Right(err) => {
                self.second.on_upgrade_error(user_data.1, err);
            }
        }
    }
}

impl<THandler1, THandler2> OutboundStreamHandler for ConnectionHandlerSelect<THandler1, THandler2>
where
    THandler1: OutboundStreamHandler,
    THandler2: OutboundStreamHandler,
{
    type OutboundUpgrade =
        Either<SendWrapper<THandler1::OutboundUpgrade>, SendWrapper<THandler2::OutboundUpgrade>>;
    type OutboundUserData = Either<THandler1::OutboundUserData, THandler2::OutboundUserData>;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::OutboundUserData,
        protocol: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        match protocol {
            future::Either::Left(output) => {
                self.first.on_fully_negotiated(
                    user_data.left().expect("Dial left info must be present"),
                    output,
                );
            }
            future::Either::Right(output) => {
                self.second.on_fully_negotiated(
                    user_data.right().expect("Dial right info must be present"),
                    output,
                );
            }
        }
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        match user_data {
            Either::Left(data) => {
                let error =
                    error.map_upgrade_err(|e| e.left().expect("Left error must be present"));
                self.first.on_upgrade_error(data, error);
            }
            Either::Right(data) => {
                let error =
                    error.map_upgrade_err(|e| e.right().expect("Right error must be present"));
                self.second.on_upgrade_error(data, error);
            }
        }
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        match self.first.poll_outbound_request(cx) {
            Poll::Ready(protocol) => {
                return Poll::Ready(
                    protocol
                        .map_upgrade(|u| Either::Left(SendWrapper(u)))
                        .map_user_data(Either::Left),
                );
            }
            Poll::Pending => {}
        }

        match self.second.poll_outbound_request(cx) {
            Poll::Ready(protocol) => {
                return Poll::Ready(
                    protocol
                        .map_upgrade(|u| Either::Right(SendWrapper(u)))
                        .map_user_data(Either::Right),
                );
            }
            Poll::Pending => {}
        }

        Poll::Pending
    }
}
