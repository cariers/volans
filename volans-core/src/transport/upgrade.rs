mod authenticate;
mod multiplex;

pub use authenticate::{Authenticate, Authenticated};
pub use multiplex::{Multiplex, Multiplexed};

use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, TryFuture, future, ready};

use crate::{
    ConnectedPoint, Listener, ListenerEvent, Multiaddr, Negotiated, PeerId, Transport,
    TransportError,
    transport::{and_then::AndThen, apply::UpgradeApplyError},
    upgrade::{
        self, InboundConnectionUpgrade, InboundUpgradeApply, OutboundConnectionUpgrade,
        OutboundUpgradeApply,
    },
};

#[derive(Clone)]
pub struct Builder<T> {
    inner: T,
}

impl<T> Builder<T>
where
    T: Transport,
    T::Error: 'static,
{
    pub fn new(inner: T) -> Builder<T> {
        Builder { inner }
    }

    /// 对传输进行身份验证。
    ///
    /// ## 转换
    ///
    ///   * I/O 升级: `C -> (PeerId, D)`.
    ///   * Transport 输出: `C -> (PeerId, D)`
    pub fn authenticate<C, D, U, E>(
        self,
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
        Authenticated::authenticate(self.inner, upgrade)
    }
}

/// 对升认证后的传输进行升级
#[derive(Debug, Clone)]
pub struct Upgrade<T, U> {
    inner: T,
    upgrade: U,
}

impl<T, U> Upgrade<T, U> {
    pub fn new(inner: T, upgrade: U) -> Self {
        Upgrade { inner, upgrade }
    }
}

impl<T, C, D, U, E> Transport for Upgrade<T, U>
where
    T: Transport<Output = (PeerId, C)>,
    T::Error: 'static,
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E>,
    U: OutboundConnectionUpgrade<Negotiated<C>, Output = D, Error = E> + Clone,
    E: std::error::Error + 'static,
{
    type Output = (PeerId, D);
    type Error = UpgradeApplyError<T::Error, E>;
    type Dial = DialUpgradeFuture<T::Dial, U, C>;
    type Incoming = ListenerUpgradeFuture<T::Incoming, U, C>;
    type Listener = UpgradeListener<T, U>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let fut = self
            .inner
            .dial(addr)
            .map_err(|e| e.map(UpgradeApplyError::Transport))?;
        Ok(DialUpgradeFuture {
            future: Box::pin(fut),
            upgrade: future::Either::Left(Some(self.upgrade.clone())),
        })
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        let inner = self
            .inner
            .listen(addr)
            .map_err(|e| e.map(UpgradeApplyError::Transport))?;

        Ok(UpgradeListener {
            inner,
            upgrade: self.upgrade.clone(),
            _phantom: PhantomData,
        })
    }
}

pub struct DialUpgradeFuture<F, U, C>
where
    U: OutboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
    future: Pin<Box<F>>,
    upgrade: future::Either<Option<U>, (PeerId, OutboundUpgradeApply<C, U>)>,
}

impl<F, U, C, D> Future for DialUpgradeFuture<F, U, C>
where
    F: TryFuture<Ok = (PeerId, C)>,
    C: AsyncRead + AsyncWrite + Unpin,
    U: OutboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = Result<(PeerId, D), UpgradeApplyError<F::Error, U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We use a `this` variable because the compiler can't mutably borrow multiple times
        // across a `Deref`.
        let this = &mut *self;

        loop {
            this.upgrade = match this.upgrade {
                future::Either::Left(ref mut up) => {
                    let (i, c) = match ready!(
                        TryFuture::try_poll(this.future.as_mut(), cx)
                            .map_err(UpgradeApplyError::Transport)
                    ) {
                        Ok(v) => v,
                        Err(err) => return Poll::Ready(Err(err)),
                    };
                    let u = up
                        .take()
                        .expect("DialUpgradeFuture is constructed with Either::Left(Some).");
                    future::Either::Right((i, upgrade::OutboundUpgradeApply::new(c, u)))
                }
                future::Either::Right((i, ref mut up)) => {
                    let d = match ready!(
                        Future::poll(Pin::new(up), cx).map_err(UpgradeApplyError::Upgrade)
                    ) {
                        Ok(d) => d,
                        Err(err) => return Poll::Ready(Err(err)),
                    };
                    return Poll::Ready(Ok((i, d)));
                }
            }
        }
    }
}

impl<F, U, C> Unpin for DialUpgradeFuture<F, U, C>
where
    U: OutboundConnectionUpgrade<Negotiated<C>>,
    C: AsyncRead + AsyncWrite + Unpin,
{
}

/// The [`Transport::ListenerUpgrade`] future of an [`Upgrade`]d transport.
pub struct ListenerUpgradeFuture<F, U, C>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
    future: Pin<Box<F>>,
    upgrade: future::Either<Option<U>, (PeerId, InboundUpgradeApply<C, U>)>,
}

impl<F, U, C, D> Future for ListenerUpgradeFuture<F, U, C>
where
    F: TryFuture<Ok = (PeerId, C)>,
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = Result<(PeerId, D), UpgradeApplyError<F::Error, U::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // We use a `this` variable because the compiler can't mutably borrow multiple times
        // across a `Deref`.
        let this = &mut *self;

        loop {
            this.upgrade = match this.upgrade {
                future::Either::Left(ref mut up) => {
                    let (i, c) = match ready!(
                        TryFuture::try_poll(this.future.as_mut(), cx)
                            .map_err(UpgradeApplyError::Transport)
                    ) {
                        Ok(v) => v,
                        Err(err) => return Poll::Ready(Err(err)),
                    };
                    let u = up
                        .take()
                        .expect("ListenerUpgradeFuture is constructed with Either::Left(Some).");
                    future::Either::Right((i, upgrade::InboundUpgradeApply::new(c, u)))
                }
                future::Either::Right((i, ref mut up)) => {
                    let d = match ready!(
                        TryFuture::try_poll(Pin::new(up), cx).map_err(UpgradeApplyError::Upgrade)
                    ) {
                        Ok(v) => v,
                        Err(err) => return Poll::Ready(Err(err)),
                    };
                    return Poll::Ready(Ok((i, d)));
                }
            }
        }
    }
}

impl<F, U, C> Unpin for ListenerUpgradeFuture<F, U, C>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>>,
{
}

#[pin_project::pin_project]
#[derive(Clone, Debug)]
pub struct UpgradeListener<T, U>
where
    T: Transport,
{
    #[pin]
    inner: T::Listener,
    upgrade: U,
    _phantom: PhantomData<T>,
}

impl<T, U, C, D> Listener for UpgradeListener<T, U>
where
    T: Transport<Output = (PeerId, C)>,
    U: Clone,
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = D>,
    U::Error: std::error::Error,
{
    type Output = (PeerId, D);
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
