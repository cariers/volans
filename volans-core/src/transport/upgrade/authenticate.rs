use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, future};

use crate::{
    ConnectedPoint, Negotiated, PeerId, StreamMuxer, Transport,
    transport::{
        and_then::AndThen,
        upgrade::{Multiplex, Multiplexed, Upgrade},
    },
    upgrade::{
        self, InboundConnectionUpgrade, InboundUpgradeApply, OutboundConnectionUpgrade,
        OutboundUpgradeApply,
    },
};

#[derive(Clone)]
pub struct Authenticated<T>(T);

impl<T> Authenticated<T> {
    pub fn authenticate<C, D, U, E>(
        transport: T,
        upgrade: U,
    ) -> Authenticated<AndThen<T, impl FnOnce(C, ConnectedPoint) -> Authenticate<C, U> + Clone>>
    where
        T: Transport<Output = C>,
        C: AsyncRead + AsyncWrite + Unpin,
        D: AsyncRead + AsyncWrite + Unpin,
        U: InboundConnectionUpgrade<Negotiated<C>, Output = (PeerId, D), Error = E>,
        U: OutboundConnectionUpgrade<Negotiated<C>, Output = (PeerId, D), Error = E> + Clone,
        E: std::error::Error + 'static,
    {
        Authenticated(transport.and_then(move |c, endpoint| Authenticate {
            inner: upgrade::apply(c, upgrade, endpoint),
        }))
    }

    pub fn apply<C, D, U, E>(self, upgrade: U) -> Authenticated<Upgrade<T, U>>
    where
        T: Transport<Output = (PeerId, C)>,
        C: AsyncRead + AsyncWrite + Unpin,
        D: AsyncRead + AsyncWrite + Unpin,
        U: InboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
        U: OutboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E> + Clone,
        E: std::error::Error + 'static,
    {
        Authenticated(Upgrade::new(self.0, upgrade))
    }

    pub fn multiplex<C, M, U, E>(
        self,
        upgrade: U,
    ) -> Multiplexed<AndThen<T, impl FnOnce((PeerId, C), ConnectedPoint) -> Multiplex<C, U> + Clone>>
    where
        T: Transport<Output = (PeerId, C)>,
        C: AsyncRead + AsyncWrite + Unpin,
        M: StreamMuxer,
        U: InboundConnectionUpgrade<Negotiated<C>, Output = M, Error = E>,
        U: OutboundConnectionUpgrade<Negotiated<C>, Output = M, Error = E> + Clone,
        E: std::error::Error + 'static,
    {
        Multiplexed::multiplex(self.0, upgrade)
    }
}

type EitherUpgrade<C, U> = future::Either<InboundUpgradeApply<C, U>, OutboundUpgradeApply<C, U>>;

#[pin_project::pin_project]
pub struct Authenticate<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>> + OutboundConnectionUpgrade<Negotiated<C>>,
{
    #[pin]
    inner: EitherUpgrade<C, U>,
}

impl<C, U> Future for Authenticate<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>
        + OutboundConnectionUpgrade<
            Negotiated<C>,
            Output = <U as InboundConnectionUpgrade<Negotiated<C>>>::Output,
            Error = <U as InboundConnectionUpgrade<Negotiated<C>>>::Error,
        >,
{
    type Output = <EitherUpgrade<C, U> as Future>::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        Future::poll(this.inner, cx)
    }
}
