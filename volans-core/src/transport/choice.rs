use std::{
    pin::Pin,
    task::{Context, Poll},
};

use either::Either;
use futures::TryFuture;
use url::Url;

use crate::{Listener, ListenerEvent, Transport, TransportError};

#[derive(Debug, Copy, Clone)]
pub struct Choice<A, B> {
    first: A,
    second: B,
}

impl<A, B> Choice<A, B> {
    pub(crate) fn new(first: A, second: B) -> Self {
        Self { first, second }
    }
}

impl<A, B> Transport for Choice<A, B>
where
    A: Transport,
    B: Transport,
{
    type Output = Either<A::Output, B::Output>;
    type Error = Either<A::Error, B::Error>;
    type Dial = ChoiceFuture<A::Dial, B::Dial>;
    type Incoming = ChoiceFuture<A::Incoming, B::Incoming>;
    type Listener = ChoiceListener<A, B>;

    fn dial(&self, addr: &Url) -> Result<Self::Dial, TransportError<Self::Error>> {
        tracing::trace!(
            address=%addr,
            "Attempting to dial using {}",
            std::any::type_name::<A>()
        );
        match self.first.dial(addr) {
            Ok(dial) => return Ok(ChoiceFuture::First(dial)),
            Err(TransportError::Other(err)) => {
                return Err(TransportError::Other(Either::Left(err)));
            }
            Err(TransportError::NotSupported(addr)) => {
                tracing::trace!(
                    address=%addr,
                    "First transport not supported, trying second"
                );
            }
        }
        tracing::trace!(
            address=%addr,
            "Attempting to dial {}",
            std::any::type_name::<B>()
        );
        match self.second.dial(addr) {
            Ok(dial) => Ok(ChoiceFuture::Second(dial)),
            Err(err) => Err(err.map(Either::Right)),
        }
    }

    fn listen(&self, addr: &Url) -> Result<Self::Listener, TransportError<Self::Error>> {
        tracing::trace!(
            address=%addr,
            "Attempting to listen using {}",
            std::any::type_name::<A>()
        );
        match self.first.listen(addr) {
            Ok(listener) => return Ok(ChoiceListener::Left(listener)),
            Err(TransportError::Other(err)) => {
                return Err(TransportError::Other(Either::Left(err)));
            }
            Err(TransportError::NotSupported(addr)) => {
                tracing::trace!(
                    address=%addr,
                    "First transport not supported, trying second"
                );
            }
        }
        tracing::trace!(
            address=%addr,
            "Attempting to listen using {}",
            std::any::type_name::<B>()
        );
        match self.second.listen(addr) {
            Ok(listener) => Ok(ChoiceListener::Right(listener)),
            Err(err) => Err(err.map(Either::Right)),
        }
    }
}

#[pin_project::pin_project(project = ChoiceListenerProj)]
pub enum ChoiceListener<A, B>
where
    A: Transport,
    B: Transport,
{
    Left(#[pin] A::Listener),
    Right(#[pin] B::Listener),
}

impl<A, B> Listener for ChoiceListener<A, B>
where
    A: Transport,
    B: Transport,
{
    type Output = Either<A::Output, B::Output>;
    type Error = Either<A::Error, B::Error>;
    type Upgrade = ChoiceFuture<A::Incoming, B::Incoming>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        match self.project() {
            ChoiceListenerProj::Left(left) => left.poll_close(cx).map_err(Either::Left),
            ChoiceListenerProj::Right(right) => right.poll_close(cx).map_err(Either::Right),
        }
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        match self.project() {
            ChoiceListenerProj::Left(left) => left
                .poll_event(cx)
                .map(|event| event.map_upgrade(ChoiceFuture::First).map_err(Either::Left)),
            ChoiceListenerProj::Right(right) => right.poll_event(cx).map(|event| {
                event
                    .map_upgrade(ChoiceFuture::Second)
                    .map_err(Either::Right)
            }),
        }
    }
}

#[derive(Debug)]
#[pin_project::pin_project(project = ChoiceFutureProj)]
pub enum ChoiceFuture<TFut1, TFut2> {
    First(#[pin] TFut1),
    Second(#[pin] TFut2),
}

impl<TFut1, TFut2, TA, TB, EA, EB> Future for ChoiceFuture<TFut1, TFut2>
where
    TFut1: TryFuture<Ok = TA, Error = EA>,
    TFut2: TryFuture<Ok = TB, Error = EB>,
{
    type Output = Result<Either<TA, TB>, Either<EA, EB>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this {
            ChoiceFutureProj::First(fut) => TryFuture::try_poll(fut, cx)
                .map_ok(Either::Left)
                .map_err(Either::Left),
            ChoiceFutureProj::Second(fut) => TryFuture::try_poll(fut, cx)
                .map_ok(Either::Right)
                .map_err(Either::Right),
        }
    }
}
