use std::{
    error, io,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{TryFutureExt, ready};

use crate::{Listener, ListenerEvent, Multiaddr, Transport, TransportError};

trait Abstract<O> {
    fn dial(&self, addr: Multiaddr) -> Result<BoxedUpgrade<O>, TransportError<io::Error>>;
    fn listen(&self, addr: Multiaddr) -> Result<BoxedListener<O>, TransportError<io::Error>>;
}

impl<T, O> Abstract<O> for T
where
    T: Transport<Output = O> + 'static,
    T::Error: Send + Sync,
    T::Dial: Send + 'static,
    T::Incoming: Send + 'static,
    T::Listener: Send + Unpin,
{
    fn dial(&self, addr: Multiaddr) -> Result<BoxedUpgrade<O>, TransportError<io::Error>> {
        let fut = Transport::dial(self, addr)
            .map_err(|e| e.map(box_err))?
            .map_err(|e| box_err(e));
        Ok(Box::pin(fut) as BoxedUpgrade<O>)
    }

    fn listen(&self, addr: Multiaddr) -> Result<BoxedListener<O>, TransportError<io::Error>> {
        let listener = Transport::listen(self, addr).map_err(|e| e.map(box_err))?;

        Ok(BoxedListener {
            inner: Box::pin(ListenerSendWrapper::new(listener)),
        })
    }
}

struct ListenerSendWrapper<L> {
    inner: L,
}

impl<L> ListenerSendWrapper<L> {
    fn new(inner: L) -> Self {
        Self { inner }
    }
}

impl<L> Listener for ListenerSendWrapper<L>
where
    L: Listener + Send + Unpin,
    L::Upgrade: Send + 'static,
    L::Error: Send + Sync + 'static,
{
    type Output = L::Output;
    type Error = io::Error;
    type Upgrade = BoxedUpgrade<L::Output>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Listener::poll_close(Pin::new(&mut self.get_mut().inner), cx).map_err(box_err)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let event = ready!(Pin::new(&mut self.get_mut().inner).poll_event(cx))
            .map_err(box_err)
            .map_upgrade(|up| Box::pin(up.map_err(box_err)) as BoxedUpgrade<L::Output>);

        Poll::Ready(event)
    }
}

type BoxedUpgrade<O> = Pin<Box<dyn Future<Output = io::Result<O>> + Send>>;

pub struct BoxedListener<O> {
    inner: Pin<Box<dyn Listener<Output = O, Error = io::Error, Upgrade = BoxedUpgrade<O>> + Send>>,
}

impl<O> Listener for BoxedListener<O> {
    type Output = O;
    type Error = io::Error;
    type Upgrade = BoxedUpgrade<O>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.get_mut().inner.as_mut().poll_close(cx)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        self.get_mut().inner.as_mut().poll_event(cx)
    }
}

pub struct Boxed<O> {
    inner: Box<dyn Abstract<O> + Send + Unpin>,
}

impl<O> Transport for Boxed<O> {
    type Output = O;
    type Error = io::Error;
    type Dial = BoxedUpgrade<O>;
    type Incoming = BoxedUpgrade<O>;
    type Listener = BoxedListener<O>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        self.inner.dial(addr)
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        self.inner.listen(addr)
    }
}

fn box_err<E: error::Error + Send + Sync + 'static>(e: E) -> io::Error {
    io::Error::other(e)
}

pub(crate) fn boxed<T>(transport: T) -> Boxed<T::Output>
where
    T: Transport + Unpin + Send + 'static,
    T::Error: Send + Sync,
    T::Dial: Send + 'static,
    T::Incoming: Send + 'static,
    T::Listener: Send + Unpin,
{
    Boxed {
        inner: Box::new(transport) as Box<_>,
    }
}
