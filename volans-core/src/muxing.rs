mod boxed;

use futures::{AsyncRead, AsyncWrite};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

pub use boxed::{StreamMuxerBox, SubstreamBox};

pub trait StreamMuxer {
    type Substream: AsyncRead + AsyncWrite;
    type Error: std::error::Error;

    /// Poll 进站子流
    fn poll_inbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>>;

    /// Poll 出站子流
    fn poll_outbound(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>>;

    /// Poll 关闭多路复用器
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;

    /// Poll 多路复用器事件
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>;
}

pub trait StreamMuxerExt: StreamMuxer + Sized {
    fn poll_inbound_unpin(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_inbound(cx)
    }

    fn poll_outbound_unpin(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<Result<Self::Substream, Self::Error>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_outbound(cx)
    }

    fn poll_unpin(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll(cx)
    }

    fn poll_close_unpin(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>>
    where
        Self: Unpin,
    {
        Pin::new(self).poll_close(cx)
    }

    fn close(self) -> Closing<Self> {
        Closing(self)
    }
}

impl<S> StreamMuxerExt for S where S: StreamMuxer {}

pub struct Closing<S>(S);

impl<S> Future for Closing<S>
where
    S: StreamMuxer + Unpin,
{
    type Output = Result<(), S::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.0.poll_close_unpin(cx)
    }
}
