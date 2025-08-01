use either::Either;
use futures::{TryFuture, future};
use pin_project::pin_project;

use std::{
    pin::Pin,
    task::{Context, Poll},
};

#[pin_project(project = EitherFutureProj)]
#[derive(Debug, Copy, Clone)]
#[must_use = "futures do nothing unless polled"]
pub enum EitherFuture<A, B> {
    First(#[pin] A),
    Second(#[pin] B),
}

impl<A, B, AOk, BOk> Future for EitherFuture<A, B>
where
    A: TryFuture<Ok = AOk>,
    B: TryFuture<Ok = BOk>,
{
    type Output = Result<future::Either<AOk, BOk>, Either<A::Error, B::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project() {
            EitherFutureProj::First(a) => match a.try_poll(cx) {
                Poll::Ready(Ok(ok)) => Poll::Ready(Ok(future::Either::Left(ok))),
                Poll::Ready(Err(err)) => Poll::Ready(Err(Either::Left(err))),
                Poll::Pending => Poll::Pending,
            },
            EitherFutureProj::Second(b) => match b.try_poll(cx) {
                Poll::Ready(Ok(ok)) => Poll::Ready(Ok(future::Either::Right(ok))),
                Poll::Ready(Err(err)) => Poll::Ready(Err(Either::Right(err))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}
