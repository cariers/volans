use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncWrite, future, ready};

use crate::{
    ConnectedPoint, Multiaddr, Negotiated, PeerId, StreamMuxer, Transport, TransportError,
    muxing::StreamMuxerBox,
    transport::{Boxed, and_then::AndThen, boxed::boxed},
    upgrade::{
        self, InboundConnectionUpgrade, InboundUpgradeApply, OutboundConnectionUpgrade,
        OutboundUpgradeApply, UpgradeError,
    },
};

#[derive(Clone)]
pub struct Multiplexed<T>(T);

impl<T> Multiplexed<T> {
    pub fn multiplex<C, M, U, E>(
        transport: T,
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
        Multiplexed(transport.and_then(move |(i, c), endpoint| {
            let upgrade = upgrade::apply(c, upgrade, endpoint);
            Multiplex {
                peer_id: Some(i),
                upgrade,
            }
        }))
    }
}
impl<T> Multiplexed<T> {
    pub fn boxed<M>(self) -> Boxed<(PeerId, StreamMuxerBox)>
    where
        T: Transport<Output = (PeerId, M)> + Sized + Send + Unpin + 'static,
        T::Dial: Send + 'static,
        T::Incoming: Send + 'static,
        T::Listener: Send + Unpin + 'static,
        T::Error: Send + Sync,
        M: StreamMuxer + Send + 'static,
        M::Substream: Send + 'static,
        M::Error: Send + Sync + 'static,
    {
        boxed(self.map(|(i, m), _| (i, StreamMuxerBox::new(m))))
    }
}

impl<T> Transport for Multiplexed<T>
where
    T: Transport,
{
    type Output = T::Output;
    type Error = T::Error;
    type Dial = T::Dial;
    type Incoming = T::Incoming;
    type Listener = T::Listener;
    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        self.0.dial(addr)
    }
    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        self.0.listen(addr)
    }
}

type EitherUpgrade<C, U> = future::Either<InboundUpgradeApply<C, U>, OutboundUpgradeApply<C, U>>;

#[pin_project::pin_project]
pub struct Multiplex<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>> + OutboundConnectionUpgrade<Negotiated<C>>,
{
    peer_id: Option<PeerId>,
    #[pin]
    upgrade: EitherUpgrade<C, U>,
}

impl<C, U, M, E> Future for Multiplex<C, U>
where
    C: AsyncRead + AsyncWrite + Unpin,
    U: InboundConnectionUpgrade<Negotiated<C>, Output = M, Error = E>,
    U: OutboundConnectionUpgrade<Negotiated<C>, Output = M, Error = E>,
{
    type Output = Result<(PeerId, M), UpgradeError<E>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let m = match ready!(Future::poll(this.upgrade, cx)) {
            Ok(m) => m,
            Err(err) => return Poll::Ready(Err(err)),
        };
        let i = this
            .peer_id
            .take()
            .expect("Multiplex future polled after completion.");
        Poll::Ready(Ok((i, m)))
    }
}
