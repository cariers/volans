use std::{
    io, iter,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, future::BoxFuture};
use volans_core::{
    PeerId, UpgradeInfo,
    identity::{PublicKey, SignatureError},
    upgrade::{InboundConnectionUpgrade, OutboundConnectionUpgrade},
};

#[derive(Clone)]
pub struct Config {
    local_pubkey: PublicKey,
}

impl Config {
    pub fn new(local_pubkey: PublicKey) -> Self {
        Self { local_pubkey }
    }

    async fn handshake<T>(self, mut socket: T) -> Result<(PeerId, IdentifyConnection<T>), Error>
    where
        T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        socket.write_all(self.local_pubkey.as_bytes()).await?;
        socket.flush().await?;
        let mut key_buf = [0; 32];
        socket.read_exact(&mut key_buf).await?;
        let remote_key = PublicKey::from_bytes(&key_buf)?;
        let peer_id = PeerId::from_bytes(remote_key.as_bytes().clone());
        Ok((peer_id, IdentifyConnection { socket, remote_key }))
    }
}

impl UpgradeInfo for Config {
    type Info = &'static str;
    type InfoIter = iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once("/v1/identify")
    }
}

impl<C> InboundConnectionUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = (PeerId, IdentifyConnection<C>);
    type Error = Error;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, _: Self::Info) -> Self::Future {
        Box::pin(self.handshake(socket))
    }
}

impl<C> OutboundConnectionUpgrade<C> for Config
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = (PeerId, IdentifyConnection<C>);
    type Error = Error;
    type Future = BoxFuture<'static, Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, _: Self::Info) -> Self::Future {
        Box::pin(self.handshake(socket))
    }
}

pub struct IdentifyConnection<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub socket: S,
    pub remote_key: PublicKey,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    SignatureError(#[from] SignatureError),
}

impl<T> AsyncRead for IdentifyConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().socket).poll_read(cx, buf)
    }

    fn poll_read_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &mut [io::IoSliceMut<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().socket).poll_read_vectored(cx, bufs)
    }
}

impl<T> AsyncWrite for IdentifyConnection<T>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.get_mut().socket).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().socket).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().socket).poll_close(cx)
    }
}
