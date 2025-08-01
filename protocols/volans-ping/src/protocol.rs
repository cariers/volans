use std::{
    io,
    time::{Duration, Instant},
};

use volans_swarm::StreamProtocol;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_NAME: StreamProtocol = StreamProtocol::new("/v1/ping");

const PING_SIZE: usize = 32;

pub(crate) async fn recv_ping<S>(mut stream: S) -> io::Result<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut payload = [0u8; PING_SIZE];

    stream.read_exact(&mut payload).await?;

    stream.write_all(&payload).await?;
    stream.flush().await?;
    Ok(stream)
}

pub(crate) async fn send_ping<S>(mut stream: S) -> io::Result<(S, Duration)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let payload: [u8; PING_SIZE] = rand::random();
    stream.write_all(&payload).await?;
    stream.flush().await?;
    let started = Instant::now();
    let mut recv_payload = [0u8; PING_SIZE];
    stream.read_exact(&mut recv_payload).await?;
    if recv_payload == payload {
        Ok((stream, started.elapsed()))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Ping payload mismatch",
        ))
    }
}
