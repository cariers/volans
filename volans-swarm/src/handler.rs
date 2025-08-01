mod dummy;
mod either;
mod map;
mod multi;
mod pending;
mod select;

pub use dummy::DummyHandler;
pub use map::{MapAction, MapEvent};
pub use pending::PendingConnectionHandler;
pub use select::ConnectionHandlerSelect;

use std::{
    fmt,
    task::{Context, Poll},
    time::Duration,
};

use ::either::Either;

use crate::{InboundUpgradeSend, OutboundUpgradeSend};

pub trait ConnectionHandler: Send + 'static {
    type Action: fmt::Debug + Send + Clone + 'static;
    type Event: fmt::Debug + Send + 'static;

    fn handle_action(&mut self, action: Self::Action);

    fn connection_keep_alive(&self) -> bool {
        false
    }

    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>>;

    fn select<H>(self, other: H) -> ConnectionHandlerSelect<Self, H>
    where
        H: ConnectionHandler,
        Self: Sized,
    {
        ConnectionHandlerSelect::select(self, other)
    }

    fn map_event<O, F>(self, map: F) -> MapEvent<Self, F>
    where
        Self: Sized,
        O: fmt::Debug + Send + 'static,
        F: FnMut(Self::Event) -> O + Send + 'static,
    {
        MapEvent::new(self, map)
    }

    fn map_action<O, F>(self, map: F) -> MapAction<Self, O, F>
    where
        Self: Sized,
        O: fmt::Debug + Send + 'static,
        F: FnMut(O) -> Option<Self::Action> + Send + 'static,
    {
        MapAction::new(self, map)
    }
}

pub trait InboundStreamHandler: ConnectionHandler {
    type InboundUpgrade: InboundUpgradeSend;
    type InboundUserData: Send + 'static;

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData>;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    );

    fn on_upgrade_error(
        &mut self,
        user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    );
}

pub trait OutboundStreamHandler: ConnectionHandler {
    type OutboundUpgrade: OutboundUpgradeSend;
    type OutboundUserData: Send + 'static;

    fn on_fully_negotiated(
        &mut self,
        user_data: Self::OutboundUserData,
        protocol: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    );

    fn on_upgrade_error(
        &mut self,
        user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    );

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>>;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SubstreamProtocol<TUpgr, TData> {
    upgrade: TUpgr,
    timeout: Duration,
    user_data: TData,
}

impl<TUpgr, TData> SubstreamProtocol<TUpgr, TData> {
    pub fn new(upgrade: TUpgr, data: TData) -> Self {
        Self {
            upgrade,
            timeout: Duration::from_secs(5),
            user_data: data,
        }
    }

    pub fn upgrade(&self) -> &TUpgr {
        &self.upgrade
    }

    pub fn timeout(&self) -> &Duration {
        &self.timeout
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn into_inner(self) -> (TUpgr, TData, Duration) {
        (self.upgrade, self.user_data, self.timeout)
    }

    pub fn map_upgrade<U, F>(self, f: F) -> SubstreamProtocol<U, TData>
    where
        F: FnOnce(TUpgr) -> U,
    {
        SubstreamProtocol {
            upgrade: f(self.upgrade),
            user_data: self.user_data,
            timeout: self.timeout,
        }
    }

    pub fn map_user_data<U, F>(self, f: F) -> SubstreamProtocol<TUpgr, U>
    where
        F: FnOnce(TData) -> U,
    {
        SubstreamProtocol {
            upgrade: self.upgrade,
            user_data: f(self.user_data),
            timeout: self.timeout,
        }
    }
}

#[derive(Debug)]
pub enum StreamUpgradeError<TUpgrErr> {
    Timeout,
    Apply(TUpgrErr),
    NegotiationFailed,
    Io(std::io::Error),
}

impl<TUpgrErr> StreamUpgradeError<TUpgrErr> {
    pub fn map_upgrade_err<F, E>(self, f: F) -> StreamUpgradeError<E>
    where
        F: FnOnce(TUpgrErr) -> E,
    {
        match self {
            StreamUpgradeError::Timeout => StreamUpgradeError::Timeout,
            StreamUpgradeError::Apply(e) => StreamUpgradeError::Apply(f(e)),
            StreamUpgradeError::NegotiationFailed => StreamUpgradeError::NegotiationFailed,
            StreamUpgradeError::Io(e) => StreamUpgradeError::Io(e),
        }
    }
}

impl<TErr1, TErr2> StreamUpgradeError<Either<TErr1, TErr2>> {
    pub fn transpose_left(self) -> StreamUpgradeError<TErr1> {
        match self {
            StreamUpgradeError::Timeout => StreamUpgradeError::Timeout,
            StreamUpgradeError::Apply(e) => {
                StreamUpgradeError::Apply(e.left().expect("StreamUpgradeError Left error expected"))
            }
            StreamUpgradeError::NegotiationFailed => StreamUpgradeError::NegotiationFailed,
            StreamUpgradeError::Io(e) => StreamUpgradeError::Io(e),
        }
    }

    fn transpose_right(self) -> StreamUpgradeError<TErr2> {
        match self {
            StreamUpgradeError::Timeout => StreamUpgradeError::Timeout,
            StreamUpgradeError::Apply(e) => StreamUpgradeError::Apply(
                e.right().expect("StreamUpgradeError Right error expected"),
            ),
            StreamUpgradeError::NegotiationFailed => StreamUpgradeError::NegotiationFailed,
            StreamUpgradeError::Io(e) => StreamUpgradeError::Io(e),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionHandlerEvent<TCustom> {
    Notify(TCustom),
    CloseConnection,
}

impl<TCustom> ConnectionHandlerEvent<TCustom> {
    pub fn map_event<O, F>(self, f: F) -> ConnectionHandlerEvent<O>
    where
        F: FnOnce(TCustom) -> O,
    {
        match self {
            ConnectionHandlerEvent::Notify(event) => ConnectionHandlerEvent::Notify(f(event)),
            ConnectionHandlerEvent::CloseConnection => ConnectionHandlerEvent::CloseConnection,
        }
    }
}
