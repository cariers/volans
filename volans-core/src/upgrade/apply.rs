use futures::{AsyncRead, AsyncWrite, future};
use std::{
    mem,
    pin::Pin,
    task::{Context, Poll},
};
use volans_stream_select::{DialerSelectFuture, ListenerSelectFuture};

use crate::{
    ConnectedPoint, Negotiated,
    upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade, UpgradeError},
};

pub fn apply<C, U>(
    socket: C,
    upgrade: U,
    connected_point: ConnectedPoint,
) -> future::Either<InboundUpgradeApply<C, U>, OutboundUpgradeApply<C, U>>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>> + OutboundConnectionUpgrade<Negotiated<C>>,
{
    match connected_point {
        ConnectedPoint::Dialer { .. } => {
            future::Either::Right(OutboundUpgradeApply::new(socket, upgrade))
        }
        _ => future::Either::Left(InboundUpgradeApply::new(socket, upgrade)),
    }
}

pub struct InboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
    inner: InboundUpgradeApplyState<C, U>,
}

#[allow(clippy::large_enum_variant)]
enum InboundUpgradeApplyState<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
    Init {
        future: ListenerSelectFuture<C, U::Info>,
        upgrade: U,
    },
    Upgrade {
        future: Pin<Box<U::Future>>,
        name: String,
    },
    Undefined,
}

impl<C, U> InboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
    pub fn new(socket: C, upgrade: U) -> Self {
        let future = ListenerSelectFuture::new(socket, upgrade.protocol_info());
        Self {
            inner: InboundUpgradeApplyState::Init { future, upgrade },
        }
    }
}

impl<C, U> Unpin for InboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
}

impl<C, U> Future for InboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
    type Output = Result<U::Output, UpgradeError<U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match mem::replace(&mut self.inner, InboundUpgradeApplyState::Undefined) {
                InboundUpgradeApplyState::Init {
                    mut future,
                    upgrade,
                } => {
                    let (info, io) = match Future::poll(Pin::new(&mut future), cx)? {
                        Poll::Ready(x) => x,
                        Poll::Pending => {
                            self.inner = InboundUpgradeApplyState::Init { future, upgrade };
                            return Poll::Pending;
                        }
                    };
                    self.inner = InboundUpgradeApplyState::Upgrade {
                        future: Box::pin(upgrade.upgrade_inbound(io, info.clone())),
                        name: info.as_ref().to_owned(),
                    };
                }
                InboundUpgradeApplyState::Upgrade { mut future, name } => {
                    match Future::poll(Pin::new(&mut future), cx) {
                        Poll::Pending => {
                            self.inner = InboundUpgradeApplyState::Upgrade { future, name };
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(x)) => {
                            tracing::trace!(upgrade=%name, "Upgraded inbound stream");
                            return Poll::Ready(Ok(x));
                        }
                        Poll::Ready(Err(e)) => {
                            tracing::debug!(upgrade=%name, "Failed to upgrade inbound stream");
                            return Poll::Ready(Err(UpgradeError::Apply(e)));
                        }
                    }
                }
                InboundUpgradeApplyState::Undefined => {
                    panic!("InboundUpgradeApplyState::poll called after completion")
                }
            }
        }
    }
}

/// Future returned by `apply_outbound`. Drives the upgrade process.
pub struct OutboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>>,
{
    inner: OutboundUpgradeApplyState<C, U>,
}

impl<C, U> OutboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>>,
{
    pub fn new(socket: C, upgrade: U) -> Self {
        let future = DialerSelectFuture::new(socket, upgrade.protocol_info());
        Self {
            inner: OutboundUpgradeApplyState::Init { future, upgrade },
        }
    }
}

enum OutboundUpgradeApplyState<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>>,
{
    Init {
        future: DialerSelectFuture<C, <U::InfoIter as IntoIterator>::IntoIter>,
        upgrade: U,
    },
    Upgrade {
        future: Pin<Box<U::Future>>,
        name: String,
    },
    Undefined,
}

impl<C, U> Unpin for OutboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>>,
{
}

impl<C, U> Future for OutboundUpgradeApply<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>>,
{
    type Output = Result<U::Output, UpgradeError<U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            match mem::replace(&mut self.inner, OutboundUpgradeApplyState::Undefined) {
                OutboundUpgradeApplyState::Init {
                    mut future,
                    upgrade,
                } => {
                    let (info, connection) = match Future::poll(Pin::new(&mut future), cx)? {
                        Poll::Ready(x) => x,
                        Poll::Pending => {
                            self.inner = OutboundUpgradeApplyState::Init { future, upgrade };
                            return Poll::Pending;
                        }
                    };
                    self.inner = OutboundUpgradeApplyState::Upgrade {
                        future: Box::pin(upgrade.upgrade_outbound(connection, info.clone())),
                        name: info.as_ref().to_owned(),
                    };
                }
                OutboundUpgradeApplyState::Upgrade { mut future, name } => {
                    match Future::poll(Pin::new(&mut future), cx) {
                        Poll::Pending => {
                            self.inner = OutboundUpgradeApplyState::Upgrade { future, name };
                            return Poll::Pending;
                        }
                        Poll::Ready(Ok(x)) => {
                            tracing::trace!(upgrade=%name, "Upgraded outbound stream");
                            return Poll::Ready(Ok(x));
                        }
                        Poll::Ready(Err(e)) => {
                            tracing::debug!(upgrade=%name, "Failed to upgrade outbound stream",);
                            return Poll::Ready(Err(UpgradeError::Apply(e)));
                        }
                    }
                }
                OutboundUpgradeApplyState::Undefined => {
                    panic!("OutboundUpgradeApplyState::poll called after completion")
                }
            }
        }
    }
}
