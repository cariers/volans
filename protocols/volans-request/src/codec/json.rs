use std::{io, marker::PhantomData};

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::{Serialize, de::DeserializeOwned};
use volans_swarm::StreamProtocol;

use crate::Codec;

#[derive(Debug, Clone)]
pub struct JsonCodec<Req, Resp> {
    request_size_maximum: u64,
    response_size_maximum: u64,
    phantom: PhantomData<(Req, Resp)>,
}

impl<Req, Resp> Default for JsonCodec<Req, Resp> {
    fn default() -> Self {
        JsonCodec {
            request_size_maximum: 1024 * 1024,
            response_size_maximum: 10 * 1024 * 1024,
            phantom: PhantomData,
        }
    }
}

impl<Req, Resp> JsonCodec<Req, Resp> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_size_maximum(mut self, size: u64) -> Self {
        self.request_size_maximum = size;
        self
    }

    pub fn response_size_maximum(mut self, size: u64) -> Self {
        self.response_size_maximum = size;
        self
    }
}

#[async_trait]
impl<Req, Resp> Codec for JsonCodec<Req, Resp>
where
    Req: Send + Serialize + DeserializeOwned,
    Resp: Send + Serialize + DeserializeOwned,
{
    type Protocol = StreamProtocol;
    type Request = Req;
    type Response = Resp;

    async fn read_request<T>(&mut self, _: &Self::Protocol, io: &mut T) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut buffer = Vec::new();
        io.take(self.request_size_maximum)
            .read_to_end(&mut buffer)
            .await?;
        Ok(serde_json::from_slice(buffer.as_slice())?)
    }

    async fn read_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let mut buffer = Vec::new();
        io.take(self.response_size_maximum)
            .read_to_end(&mut buffer)
            .await?;
        Ok(serde_json::from_slice(buffer.as_slice())?)
    }

    async fn write_request<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        request: Self::Request,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = serde_json::to_vec(&request)?;
        io.write_all(&data).await?;
        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _: &Self::Protocol,
        io: &mut T,
        response: Self::Response,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let data = serde_json::to_vec(&response)?;
        io.write_all(&data).await?;
        Ok(())
    }
}
