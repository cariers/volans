pub(crate) mod boxed;

pub mod and_then;
pub mod apply;
pub mod choice;
pub mod map;
pub mod map_err;
pub mod timeout;
pub mod upgrade;

pub use boxed::{Boxed, BoxedListener};

use futures::TryFuture;

use std::{
    error, fmt,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use crate::{
    ConnectedPoint, Multiaddr, Negotiated,
    upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade},
};

pub trait Listener {
    type Output;
    type Error: std::error::Error;
    type Upgrade: Future<Output = Result<Self::Output, Self::Error>>;

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
}
pub trait Transport {
    type Output;
    type Error: std::error::Error;
    type Dial: Future<Output = Result<Self::Output, Self::Error>>;
    type Incoming: Future<Output = Result<Self::Output, Self::Error>>;
    type Listener: Listener<Output = Self::Output, Error = Self::Error, Upgrade = Self::Incoming>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>>;
    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>>;

    fn and_then<D, TMap, TMapFut>(self, map: TMap) -> and_then::AndThen<Self, TMap>
    where
        Self: Sized,
        TMap: FnOnce(Self::Output, ConnectedPoint) -> TMapFut + Clone,
        TMapFut: TryFuture<Ok = D>,
        TMapFut::Error: std::error::Error,
    {
        and_then::AndThen::new(self, map)
    }

    fn map<D, TMap>(self, map: TMap) -> map::Map<Self, TMap>
    where
        Self: Sized,
        TMap: FnOnce(Self::Output, ConnectedPoint) -> D + Clone,
    {
        map::Map::new(self, map)
    }

    fn map_err<TErr, F>(self, map: F) -> map_err::MapErr<Self, F>
    where
        Self: Sized,
        F: FnOnce(Self::Error) -> TErr + Clone,
        TErr: std::error::Error,
    {
        map_err::MapErr::new(self, map)
    }

    fn apply<C, D, E, U>(self, upgrade: U) -> apply::UpgradeApply<Self, U>
    where
        Self: Sized,
        Self: Transport<Output = C>,
        U: InboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
        U: OutboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
        E: std::error::Error,
    {
        apply::UpgradeApply::new(self, upgrade)
    }

    fn choice<B>(self, other: B) -> choice::Choice<Self, B>
    where
        Self: Sized,
        B: Transport,
    {
        choice::Choice::new(self, other)
    }

    fn timeout(self, timeout: Duration) -> timeout::Timeout<Self>
    where
        Self: Sized,
    {
        timeout::Timeout::new(self, timeout)
    }

    fn boxed(self) -> boxed::Boxed<Self::Output>
    where
        Self: Sized + Send + Unpin + 'static,
        Self::Dial: Send + 'static,
        Self::Incoming: Send + 'static,
        Self::Error: Send + Sync,
        Self::Listener: Send + Unpin + 'static,
    {
        boxed::boxed(self)
    }

    fn upgrade(self) -> upgrade::Builder<Self>
    where
        Self: Sized,
        Self::Error: 'static,
    {
        upgrade::Builder::new(self)
    }
}

pub enum ListenerEvent<TUpgr, TErr> {
    NewAddress(Multiaddr),
    AddressExpired(Multiaddr),
    Incoming {
        local_addr: Multiaddr,
        remote_addr: Multiaddr,
        upgrade: TUpgr,
    },
    Closed(Result<(), TErr>),
    Error(TErr),
}

impl<TUpgr, TErr> ListenerEvent<TUpgr, TErr> {
    pub fn map_upgrade<F, TUpgr2>(self, map: F) -> ListenerEvent<TUpgr2, TErr>
    where
        F: FnOnce(TUpgr) -> TUpgr2,
    {
        match self {
            ListenerEvent::NewAddress(addr) => ListenerEvent::NewAddress(addr),
            ListenerEvent::AddressExpired(addr) => ListenerEvent::AddressExpired(addr),
            ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade,
            } => ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade: map(upgrade),
            },
            ListenerEvent::Closed(res) => ListenerEvent::Closed(res),
            ListenerEvent::Error(err) => ListenerEvent::Error(err),
        }
    }

    pub fn map_err<F, TErr2>(self, map: F) -> ListenerEvent<TUpgr, TErr2>
    where
        F: FnOnce(TErr) -> TErr2,
    {
        match self {
            ListenerEvent::NewAddress(addr) => ListenerEvent::NewAddress(addr),
            ListenerEvent::AddressExpired(addr) => ListenerEvent::AddressExpired(addr),
            ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade,
            } => ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade,
            },
            ListenerEvent::Closed(res) => ListenerEvent::Closed(res.map_err(map)),
            ListenerEvent::Error(err) => ListenerEvent::Error(map(err)),
        }
    }
}

#[derive(Debug, Clone)]
pub enum TransportError<TErr> {
    NotSupported(Multiaddr),
    Other(TErr),
}

impl<TErr> From<TErr> for TransportError<TErr> {
    fn from(err: TErr) -> Self {
        TransportError::Other(err)
    }
}

impl<TErr> TransportError<TErr> {
    pub fn map<E, F>(self, map: F) -> TransportError<E>
    where
        F: FnOnce(TErr) -> E,
        E: std::error::Error,
    {
        match self {
            TransportError::NotSupported(addr) => TransportError::NotSupported(addr),
            TransportError::Other(err) => TransportError::Other(map(err)),
        }
    }
}

impl<TErr> fmt::Display for TransportError<TErr>
where
    TErr: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::NotSupported(addr) => {
                write!(f, "Transport not supported for address: {}", addr)
            }
            TransportError::Other(err) => write!(f, "Transport error: {}", err),
        }
    }
}

impl<TErr> error::Error for TransportError<TErr>
where
    TErr: error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            TransportError::NotSupported(_) => None,
            TransportError::Other(err) => Some(err),
        }
    }
}
