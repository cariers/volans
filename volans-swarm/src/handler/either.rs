use std::task::{Context, Poll};

use either::Either;
use futures::future;

use crate::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    OutboundStreamHandler, OutboundUpgradeSend, StreamUpgradeError, SubstreamProtocol,
    upgrade::SendWrapper,
};

impl<L, R> ConnectionHandler for Either<L, R>
where
    L: ConnectionHandler,
    R: ConnectionHandler,
{
    type Action = Either<L::Action, R::Action>;
    type Event = Either<L::Event, R::Event>;

    fn handle_action(&mut self, action: Self::Action) {
        match (self, action) {
            (Either::Left(left), Either::Left(action)) => left.handle_action(action),
            (Either::Right(right), Either::Right(action)) => right.handle_action(action),
            _ => unreachable!(),
        }
    }

    fn connection_keep_alive(&self) -> bool {
        match self {
            Either::Left(left) => left.connection_keep_alive(),
            Either::Right(right) => right.connection_keep_alive(),
        }
    }

    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        match self {
            Either::Left(left) => left.poll(cx).map(|e| e.map_event(Either::Left)),
            Either::Right(right) => right.poll(cx).map(|e| e.map_event(Either::Right)),
        }
    }
}

impl<L, R> InboundStreamHandler for Either<L, R>
where
    L: InboundStreamHandler,
    R: InboundStreamHandler,
{
    type InboundUpgrade = Either<SendWrapper<L::InboundUpgrade>, SendWrapper<R::InboundUpgrade>>;
    type InboundUserData = Either<L::InboundUserData, R::InboundUserData>;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        match self {
            Either::Left(left) => left
                .listen_protocol()
                .map_upgrade(|u| Either::Left(SendWrapper(u)))
                .map_user_data(Either::Left),
            Either::Right(right) => right
                .listen_protocol()
                .map_upgrade(|u| Either::Right(SendWrapper(u)))
                .map_user_data(Either::Right),
        }
    }

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        match (self, user_data, protocol) {
            (Either::Left(left), Either::Left(data), future::Either::Left(protocol)) => {
                left.on_fully_negotiated(data, protocol)
            }
            (Either::Right(right), Either::Right(data), future::Either::Right(protocol)) => {
                right.on_fully_negotiated(data, protocol)
            }
            (_, _, _) => unreachable!("Invalid fully negotiated protocol for either handler"),
        }
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        match (self, user_data, error) {
            (Either::Left(left), Either::Left(data), Either::Left(error)) => {
                left.on_upgrade_error(data, error)
            }
            (Either::Right(right), Either::Right(data), Either::Right(error)) => {
                right.on_upgrade_error(data, error)
            }
            (_, _, _) => unreachable!("Invalid upgrade error for either handler"),
        }
    }
}

impl<L, R> OutboundStreamHandler for Either<L, R>
where
    L: OutboundStreamHandler,
    R: OutboundStreamHandler,
{
    type OutboundUpgrade = Either<SendWrapper<L::OutboundUpgrade>, SendWrapper<R::OutboundUpgrade>>;
    type OutboundUserData = Either<L::OutboundUserData, R::OutboundUserData>;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::OutboundUserData,
        protocol: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        match (self, user_data, protocol) {
            (Either::Left(left), Either::Left(data), future::Either::Left(protocol)) => {
                left.on_fully_negotiated(data, protocol)
            }
            (Either::Right(right), Either::Right(data), future::Either::Right(protocol)) => {
                right.on_fully_negotiated(data, protocol)
            }
            (_, _, _) => unreachable!("Invalid fully negotiated protocol for either handler"),
        }
    }

    fn on_upgrade_error(
        &mut self,
        user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        match (self, user_data, error) {
            (Either::Left(left), Either::Left(data), error) => {
                left.on_upgrade_error(data, error.transpose_left())
            }
            (Either::Right(right), Either::Right(data), error) => {
                right.on_upgrade_error(data, error.transpose_right())
            }
            (_, _, _) => unreachable!("Invalid upgrade error for either handler"),
        }
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        match self {
            Either::Left(left) => left.poll_outbound_request(cx).map(|p| {
                p.map_upgrade(|u| Either::Left(SendWrapper(u)))
                    .map_user_data(Either::Left)
            }),
            Either::Right(right) => right.poll_outbound_request(cx).map(|p| {
                p.map_upgrade(|u| Either::Right(SendWrapper(u)))
                    .map_user_data(Either::Right)
            }),
        }
    }
}
