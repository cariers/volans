use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use either::Either;
use futures::TryFuture;

use crate::{ConnectedPoint, Listener, ListenerEvent, Multiaddr, Transport, TransportError};

#[derive(Debug, Copy, Clone)]
pub struct AndThen<T, TMap> {
    transport: T,
    map: TMap,
}

impl<T, TMap> AndThen<T, TMap> {
    pub(crate) fn new(transport: T, fun: TMap) -> Self {
        AndThen {
            transport,
            map: fun,
        }
    }
}

impl<O, T, TMap, TMapFut> Transport for AndThen<T, TMap>
where
    T: Transport,
    TMap: FnOnce(T::Output, ConnectedPoint) -> TMapFut + Clone,
    TMapFut: TryFuture<Ok = O>,
    TMapFut::Error: std::error::Error,
{
    type Output = O;
    type Error = Either<T::Error, TMapFut::Error>;
    type Dial = AndThenFuture<T::Dial, TMap, TMapFut>;
    type Incoming = AndThenFuture<T::Incoming, TMap, TMapFut>;
    type Listener = AndThenListener<T, TMap>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        match self.transport.dial(addr.clone()) {
            Ok(dial) => Ok(AndThenFuture {
                inner: Either::Left(Box::pin(dial)),
                args: Some((self.map.clone(), ConnectedPoint::Dialer { addr })),
            }),
            Err(err) => Err(err.map(Either::Left)),
        }
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        match self.transport.listen(addr) {
            Ok(listener) => Ok(AndThenListener {
                inner: listener,
                map: self.map.clone(),
                _phantom: PhantomData,
            }),
            Err(err) => Err(err.map(Either::Left)),
        }
    }
}

#[pin_project::pin_project]
#[derive(Clone, Debug)]
pub struct AndThenListener<T, TMap>
where
    T: Transport,
{
    #[pin]
    inner: T::Listener,
    map: TMap,
    _phantom: PhantomData<T>,
}

impl<O, T, TMap, TMapFut> Listener for AndThenListener<T, TMap>
where
    T: Transport,
    TMap: FnOnce(T::Output, ConnectedPoint) -> TMapFut + Clone,
    TMapFut: TryFuture<Ok = O>,
    TMapFut::Error: std::error::Error,
{
    type Output = O;
    type Error = Either<T::Error, TMapFut::Error>;
    type Upgrade = AndThenFuture<T::Incoming, TMap, TMapFut>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_close(cx).map_err(Either::Left)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.project();
        this.inner.poll_event(cx).map(|event| match event {
            ListenerEvent::Incoming {
                local_addr,
                remote_addr,
                upgrade,
            } => ListenerEvent::Incoming {
                local_addr: local_addr.clone(),
                remote_addr: remote_addr.clone(),
                upgrade: AndThenFuture {
                    inner: Either::Left(Box::pin(upgrade)),
                    args: Some((
                        this.map.clone(),
                        ConnectedPoint::Listener {
                            local_addr,
                            remote_addr,
                        },
                    )),
                },
            },
            ListenerEvent::Closed(cause) => ListenerEvent::Closed(cause.map_err(Either::Left)),
            ListenerEvent::Listened(addr) => ListenerEvent::Listened(addr),
            ListenerEvent::Error(err) => ListenerEvent::Error(Either::Left(err)),
        })
    }
}

#[derive(Debug)]
pub struct AndThenFuture<TFut, TMap, TMapFut> {
    inner: Either<Pin<Box<TFut>>, Pin<Box<TMapFut>>>,
    args: Option<(TMap, ConnectedPoint)>,
}

impl<TFut, TMap, TMapFut> Unpin for AndThenFuture<TFut, TMap, TMapFut> {}

impl<TFut, TMap, TMapFut> Future for AndThenFuture<TFut, TMap, TMapFut>
where
    TFut: TryFuture,
    TMapFut: TryFuture,
    TMap: FnOnce(TFut::Ok, ConnectedPoint) -> TMapFut,
{
    type Output = Result<TMapFut::Ok, Either<TFut::Error, TMapFut::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            let future = match &mut self.inner {
                Either::Left(fut) => {
                    let output = match fut.as_mut().try_poll(cx) {
                        Poll::Ready(Ok(v)) => v,
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(Either::Left(e))),
                        Poll::Pending => return Poll::Pending,
                    };
                    let (map, connection_point) = self.args.take().expect("args should be set");
                    map(output, connection_point)
                }
                Either::Right(fut) => {
                    return match fut.as_mut().try_poll(cx) {
                        Poll::Ready(Ok(v)) => Poll::Ready(Ok(v)),
                        Poll::Ready(Err(e)) => Poll::Ready(Err(Either::Right(e))),
                        Poll::Pending => Poll::Pending,
                    };
                }
            };
            self.inner = Either::Right(Box::pin(future));
        }
    }
}
