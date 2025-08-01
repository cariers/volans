use std::{
    pin::Pin,
    task::{Context, Poll},
};

use url::Url;

use crate::{Listener, ListenerEvent, Transport, TransportError};

#[derive(Debug, Copy, Clone)]
pub struct MapErr<T, F> {
    transport: T,
    map: F,
}

impl<T, F> MapErr<T, F> {
    pub(crate) fn new(transport: T, map: F) -> Self {
        Self { transport, map }
    }
}

impl<T, F, TErr> Transport for MapErr<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> TErr + Clone,
    TErr: std::error::Error,
{
    type Output = T::Output;
    type Error = TErr;
    type Dial = MapErrDial<T, F>;
    type Incoming = MapErrUpgrade<T, F>;
    type Listener = MapErrListener<T, F>;

    fn dial(&self, addr: &Url) -> Result<Self::Dial, TransportError<Self::Error>> {
        let map = self.map.clone();

        match self.transport.dial(addr) {
            Ok(dial) => Ok(MapErrDial {
                inner: dial,
                map: Some(map),
            }),
            Err(err) => Err(err.map(map)),
        }
    }

    fn listen(&self, addr: &Url) -> Result<Self::Listener, TransportError<Self::Error>> {
        let map = self.map.clone();
        match self.transport.listen(addr) {
            Ok(listener) => Ok(MapErrListener {
                inner: listener,
                map,
            }),
            Err(err) => Err(err.map(map)),
        }
    }
}

#[pin_project::pin_project]
pub struct MapErrDial<T: Transport, F> {
    #[pin]
    inner: T::Dial,
    map: Option<F>,
}

impl<T, F, TErr> Future for MapErrDial<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> TErr + Clone,
    TErr: std::error::Error,
{
    type Output = Result<T::Output, TErr>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Ready(Ok(output)) => Poll::Ready(Ok(output)),
            Poll::Ready(Err(err)) => {
                let map = this.map.take().expect("MapErrDial can only be polled once");
                Poll::Ready(Err(map(err)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[pin_project::pin_project]
pub struct MapErrUpgrade<T: Transport, F> {
    #[pin]
    inner: T::Incoming,
    map: Option<F>,
}

impl<T, F, TErr> Future for MapErrUpgrade<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> TErr + Clone,
    TErr: std::error::Error,
{
    type Output = Result<T::Output, TErr>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Ready(Ok(output)) => Poll::Ready(Ok(output)),
            Poll::Ready(Err(err)) => {
                let map = this
                    .map
                    .take()
                    .expect("MapErrUpgrade can only be polled once");
                Poll::Ready(Err(map(err)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[pin_project::pin_project]
pub struct MapErrListener<T: Transport, F> {
    #[pin]
    inner: T::Listener,
    map: F,
}

impl<T, F, TErr> Listener for MapErrListener<T, F>
where
    T: Transport,
    F: FnOnce(T::Error) -> TErr + Clone,
    TErr: std::error::Error,
{
    type Output = T::Output;
    type Error = TErr;
    type Upgrade = MapErrUpgrade<T, F>;

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, TErr>> {
        let this = self.project();
        let map = &*this.map;
        this.inner.poll_event(cx).map(|ev| {
            ev.map_upgrade(move |u| MapErrUpgrade {
                inner: u,
                map: Some(map.clone()),
            })
            .map_err(map.clone())
        })
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), TErr>> {
        let this = self.project();
        match this.inner.poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(err)) => {
                let map = this.map.clone();
                Poll::Ready(Err(map(err)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
