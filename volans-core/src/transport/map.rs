use crate::{ConnectedPoint, Listener, ListenerEvent, Multiaddr, Transport, TransportError};
use futures::TryFuture;
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

#[derive(Debug, Copy, Clone)]
pub struct Map<T, F> {
    transport: T,
    map: F,
}

impl<T, TMap> Map<T, TMap> {
    pub(crate) fn new(transport: T, map: TMap) -> Self {
        Map { transport, map }
    }

    pub fn inner(&self) -> &T {
        &self.transport
    }
}

impl<T, D, TMap> Transport for Map<T, TMap>
where
    T: Transport,
    TMap: FnOnce(T::Output, ConnectedPoint) -> D + Clone,
{
    type Output = D;
    type Error = T::Error;
    type Dial = MapFuture<T::Dial, TMap>;
    type Incoming = MapFuture<T::Incoming, TMap>;
    type Listener = MapListener<T, TMap>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        match self.transport.dial(addr.clone()) {
            Ok(dial) => Ok(MapFuture {
                inner: dial,
                args: Some((self.map.clone(), ConnectedPoint::Dialer { addr })),
            }),
            Err(err) => Err(err),
        }
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        match self.transport.listen(addr) {
            Ok(listener) => Ok(MapListener {
                inner: listener,
                fun: self.map.clone(),
                _phantom: PhantomData,
            }),
            Err(err) => Err(err),
        }
    }
}

#[pin_project::pin_project]
#[derive(Clone, Debug)]
pub struct MapListener<T, TMap>
where
    T: Transport,
{
    #[pin]
    inner: T::Listener,
    fun: TMap,
    _phantom: PhantomData<T>,
}

impl<D, T, TMap> Listener for MapListener<T, TMap>
where
    T: Transport,
    TMap: FnOnce(T::Output, ConnectedPoint) -> D + Clone,
{
    type Output = D;
    type Error = T::Error;
    type Upgrade = MapFuture<T::Incoming, TMap>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_close(cx)
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
                upgrade: MapFuture {
                    inner: upgrade,
                    args: Some((
                        this.fun.clone(),
                        ConnectedPoint::Listener {
                            local_addr,
                            remote_addr,
                        },
                    )),
                },
            },
            ListenerEvent::Closed(cause) => ListenerEvent::Closed(cause),
            ListenerEvent::Listened(addr) => ListenerEvent::Listened(addr),
            ListenerEvent::Error(err) => ListenerEvent::Error(err),
        })
    }
}

#[pin_project::pin_project]
#[derive(Debug)]
pub struct MapFuture<T, F> {
    #[pin]
    inner: T,
    args: Option<(F, ConnectedPoint)>,
}

impl<T, A, F, B> Future for MapFuture<T, F>
where
    T: TryFuture<Ok = A>,
    F: FnOnce(A, ConnectedPoint) -> B,
{
    type Output = Result<B, T::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let item = match TryFuture::try_poll(this.inner, cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(v)) => v,
            Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
        };
        let (f, a) = this.args.take().expect("MapFuture has already finished.");
        Poll::Ready(Ok(f(item, a)))
    }
}
