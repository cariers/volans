use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, TryFuture, future, ready};
use url::Url;

use crate::{
    Listener, ListenerEvent, Negotiated, Transport, TransportError,
    upgrade::{
        InboundConnectionUpgrade, InboundUpgradeApply, OutboundConnectionUpgrade,
        OutboundUpgradeApply, UpgradeError,
    },
};

#[derive(Debug, Copy, Clone)]
pub struct UpgradeApply<T, U> {
    transport: T,
    upgrade: U,
}

impl<T, U> UpgradeApply<T, U> {
    pub(crate) fn new(transport: T, upgrade: U) -> Self {
        UpgradeApply { transport, upgrade }
    }
}

impl<T, C, D, E, U> Transport for UpgradeApply<T, U>
where
    T: Transport<Output = C>,
    C: AsyncRead + AsyncWrite + Unpin,
    U: Clone,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
    U: OutboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
    E: std::error::Error,
{
    type Output = D;
    type Error = UpgradeApplyError<T::Error, E>;
    type Dial = DialUpgradeFuture<T::Dial, U, C>;
    type Incoming = ListenerUpgradeFuture<T::Incoming, U, C>;
    type Listener = UpgradeApplyListener<T, U>;

    fn dial(&self, addr: &Url) -> Result<Self::Dial, TransportError<Self::Error>> {
        let fut = self
            .transport
            .dial(addr)
            .map_err(|e| e.map(UpgradeApplyError::Transport))?;
        Ok(DialUpgradeFuture {
            future: Box::pin(fut),
            upgrade: future::Either::Left(Some(self.upgrade.clone())),
        })
    }

    fn listen(&self, addr: &Url) -> Result<Self::Listener, TransportError<Self::Error>> {
        let inner = self
            .transport
            .listen(addr)
            .map_err(|e| e.map(UpgradeApplyError::Transport))?;

        Ok(UpgradeApplyListener {
            inner,
            upgrade: self.upgrade.clone(),
            _phantom: PhantomData,
        })
    }
}

#[pin_project::pin_project]
#[derive(Clone, Debug)]
pub struct UpgradeApplyListener<T, U>
where
    T: Transport,
{
    #[pin]
    inner: T::Listener,
    upgrade: U,
    _phantom: PhantomData<T>,
}

impl<T, U, C, D> Listener for UpgradeApplyListener<T, U>
where
    T: Transport<Output = C>,
    U: Clone,
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = D;
    type Error = UpgradeApplyError<T::Error, U::Error>;
    type Upgrade = ListenerUpgradeFuture<T::Incoming, U, C>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner
            .poll_close(cx)
            .map_err(UpgradeApplyError::Transport)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.project();
        this.inner.poll_event(cx).map(|event| {
            event
                .map_upgrade(move |u| ListenerUpgradeFuture {
                    future: Box::pin(u),
                    upgrade: future::Either::Left(Some(this.upgrade.clone())),
                })
                .map_err(UpgradeApplyError::Transport)
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpgradeApplyError<TErr, TUpgrErr> {
    #[error("Transport error: {0}")]
    Transport(TErr),
    #[error("Upgrade error: {0}")]
    Upgrade(UpgradeError<TUpgrErr>),
}

pub struct ListenerUpgradeFuture<F, U, C>
where
    U: InboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
    future: Pin<Box<F>>,
    upgrade: future::Either<Option<U>, InboundUpgradeApply<C, U>>,
}

impl<F, U, C> Unpin for ListenerUpgradeFuture<F, U, C>
where
    U: InboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
}

impl<F, U, C, D> Future for ListenerUpgradeFuture<F, U, C>
where
    F: TryFuture<Ok = C>,
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = Result<D, UpgradeApplyError<F::Error, U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        loop {
            this.upgrade = match this.upgrade {
                future::Either::Left(ref mut upgrade) => {
                    let c = ready!(
                        TryFuture::try_poll(this.future.as_mut(), cx)
                            .map_err(UpgradeApplyError::Transport)?
                    );
                    let u = upgrade
                        .take()
                        .expect("ListenerUpgradeFuture should have upgrade set");
                    // 使用 InboundUpgradeApply to apply the upgrade
                    future::Either::Right(InboundUpgradeApply::new(c, u))
                }
                future::Either::Right(ref mut upgrade) => {
                    let res = ready!(
                        Future::poll(Pin::new(upgrade), cx).map_err(UpgradeApplyError::Upgrade)?
                    );
                    return Poll::Ready(Ok(res));
                }
            }
        }
    }
}

pub struct DialUpgradeFuture<F, U, C>
where
    U: OutboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
    future: Pin<Box<F>>,
    upgrade: future::Either<Option<U>, OutboundUpgradeApply<C, U>>,
}

impl<F, U, C> Unpin for DialUpgradeFuture<F, U, C>
where
    U: OutboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
}

impl<F, U, C, D> Future for DialUpgradeFuture<F, U, C>
where
    F: TryFuture<Ok = C>,
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = Result<D, UpgradeApplyError<F::Error, U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        loop {
            this.upgrade = match this.upgrade {
                // 上层 Upgrade
                future::Either::Left(ref mut upgrade) => {
                    let c = ready!(
                        TryFuture::try_poll(this.future.as_mut(), cx)
                            .map_err(UpgradeApplyError::Transport)?
                    );
                    let u = upgrade
                        .take()
                        .expect("DialUpgradeFuture should have upgrade set");
                    // 使用 OutboundUpgradeApply to apply the upgrade
                    future::Either::Right(OutboundUpgradeApply::new(c, u))
                }
                future::Either::Right(ref mut upgrade) => {
                    let res = ready!(
                        TryFuture::try_poll(Pin::new(upgrade), cx)
                            .map_err(UpgradeApplyError::Upgrade)?
                    );
                    return Poll::Ready(Ok(res));
                }
            }
        }
    }
}
