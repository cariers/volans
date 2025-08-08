use std::{io, marker::PhantomData};

use async_trait::async_trait;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use volans_swarm::StreamProtocol;

use crate::Codec;

#[derive(Debug, Clone)]
pub struct ProtobufCodec<Req, Resp> {
    request_size_maximum: u64,
    response_size_maximum: u64,
    phantom: PhantomData<(Req, Resp)>,
}

impl<Req, Resp> Default for ProtobufCodec<Req, Resp> {
    fn default() -> Self {
        ProtobufCodec {
            request_size_maximum: 1024 * 1024,
            response_size_maximum: 10 * 1024 * 1024,
            phantom: PhantomData,
        }
    }
}

impl<Req, Resp> ProtobufCodec<Req, Resp> {
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
impl<Req, Resp> Codec for ProtobufCodec<Req, Resp>
where
    Req: prost::Message + Send + Default,
    Resp: prost::Message + Send + Default,
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
        Ok(prost::Message::decode(buffer.as_slice())?)
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
        Ok(prost::Message::decode(buffer.as_slice())?)
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
        let data = request.encode_to_vec();
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
        let data = response.encode_to_vec();
        io.write_all(&data).await?;
        Ok(())
    }
}
