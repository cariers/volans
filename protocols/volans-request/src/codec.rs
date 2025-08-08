#[cfg(feature = "json")]
mod json;

#[cfg(feature = "json")]
pub use json::JsonCodec;

#[cfg(feature = "protobuf")]
mod protobuf;

#[cfg(feature = "protobuf")]
pub use protobuf::ProtobufCodec;

use std::io;

use async_trait::async_trait;
use futures::{AsyncRead, AsyncWrite};

#[async_trait]
pub trait Codec {
    type Protocol: AsRef<str> + Send + Clone;
    type Request: Send;
    type Response: Send;

    async fn read_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send;
    async fn read_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send;
    async fn write_request<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send;
    async fn write_response<T>(
        &mut self,
        protocol: &Self::Protocol,
        io: &mut T,
        response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send;
}
