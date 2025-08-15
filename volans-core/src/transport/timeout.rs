use std::{
    error, fmt,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::TryFuture;
use futures_timer::Delay;

use crate::{Listener, Multiaddr, Transport, TransportError};

#[derive(Debug, Clone)]
pub struct Timeout<T> {
    inner: T,
    outgoing_timeout: Duration,
    incoming_timeout: Duration,
}

impl<T> Timeout<T> {
    pub fn new(inner: T, timeout: Duration) -> Self {
        Self {
            inner,
            outgoing_timeout: timeout,
            incoming_timeout: timeout,
        }
    }

    pub fn outgoing_timeout(&self) -> Duration {
        self.outgoing_timeout
    }

    pub fn incoming_timeout(&self) -> Duration {
        self.incoming_timeout
    }

    pub fn with_outgoing_timeout(mut self, timeout: Duration) -> Self {
        self.outgoing_timeout = timeout;
        self
    }

    pub fn with_incoming_timeout(mut self, timeout: Duration) -> Self {
        self.incoming_timeout = timeout;
        self
    }
}

impl<T> Transport for Timeout<T>
where
    T: Transport,
    T::Error: 'static,
{
    type Output = T::Output;
    type Error = TimeoutError<T::Error>;
    type Dial = TimeoutFuture<T::Dial>;
    type Incoming = TimeoutFuture<T::Incoming>;
    type Listener = TimeoutListener<T>;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let fut = self
            .inner
            .dial(addr)
            .map_err(|e| e.map(TimeoutError::Other))?;
        Ok(TimeoutFuture {
            inner: fut,
            timer: Delay::new(self.outgoing_timeout),
        })
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        let listener = self
            .inner
            .listen(addr)
            .map_err(|e| e.map(TimeoutError::Other))?;
        Ok(TimeoutListener {
            inner: listener,
            timeout: self.incoming_timeout,
            _marker: PhantomData,
        })
    }
}

#[pin_project::pin_project]
pub struct TimeoutListener<T>
where
    T: Transport,
{
    #[pin]
    inner: T::Listener,
    timeout: Duration,
    _marker: PhantomData<T>,
}

impl<T> Listener for TimeoutListener<T>
where
    T: Transport,
    T::Error: 'static,
{
    type Output = T::Output;
    type Error = TimeoutError<T::Error>;
    type Upgrade = TimeoutFuture<T::Incoming>;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_close(cx).map_err(TimeoutError::Other)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<super::ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.project();
        let timeout = *this.timeout;
        this.inner.poll_event(cx).map(|event| {
            event
                .map_upgrade(move |u| TimeoutFuture {
                    inner: u,
                    timer: Delay::new(timeout),
                })
                .map_err(TimeoutError::Other)
        })
    }
}

#[pin_project::pin_project]
pub struct TimeoutFuture<TFut> {
    #[pin]
    inner: TFut,
    timer: Delay,
}

impl<TFut> Future for TimeoutFuture<TFut>
where
    TFut: TryFuture,
{
    type Output = Result<TFut::Ok, TimeoutError<TFut::Error>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        match TryFuture::try_poll(this.inner, cx) {
            Poll::Pending => {}
            Poll::Ready(Ok(v)) => return Poll::Ready(Ok(v)),
            Poll::Ready(Err(err)) => return Poll::Ready(Err(TimeoutError::Other(err))),
        }
        // 检查是否超时
        match Pin::new(&mut this.timer).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(()) => Poll::Ready(Err(TimeoutError::Timeout)),
        }
    }
}

#[derive(Debug)]
pub enum TimeoutError<TErr> {
    Timeout,
    Other(TErr),
}

impl<TErr> fmt::Display for TimeoutError<TErr>
where
    TErr: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeoutError::Timeout => write!(f, "Operation timed out"),
            TimeoutError::Other(err) => write!(f, "Other error: {}", err),
        }
    }
}

impl<TErr> error::Error for TimeoutError<TErr>
where
    TErr: error::Error + 'static,
{
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            TimeoutError::Timeout => None,
            TimeoutError::Other(err) => Some(err),
        }
    }
}
