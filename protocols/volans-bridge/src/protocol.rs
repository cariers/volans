use std::{io, str::FromStr};

use futures::{SinkExt, StreamExt};
use volans_codec::{Bytes, Framed, FramedParts, ProtobufUviCodec};
use volans_core::{Multiaddr, PeerId};
use volans_swarm::{StreamProtocol, Substream};

pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/volans.bridge.v1.rs"));
}

pub(crate) const PROTOCOL_NAME: StreamProtocol = StreamProtocol::new("/v1/bridge");

const MAX_MESSAGE_SIZE: usize = 1024; // 1 MB

pub(crate) async fn make_bridge_connect(
    io: Substream,
    dst_peer_id: PeerId,
    addresses: Vec<Multiaddr>,
) -> Result<(Substream, Bytes), ConnectError> {
    let mut framed = Framed::new(
        io,
        ProtobufUviCodec::<v1::BridgeConnect>::new(MAX_MESSAGE_SIZE),
    );
    let message = v1::BridgeConnect {
        peer: Some(v1::Peer {
            id: dst_peer_id.into_base58(),
            addresses: addresses.into_iter().map(|a| a.to_string()).collect(),
        }),
    };
    // 写入连接消息
    framed.send(message).await?;
    framed.flush().await?;

    let parts = framed
        .into_parts()
        .map_codec(|_| ProtobufUviCodec::<v1::BridgeStatus>::new(MAX_MESSAGE_SIZE));

    let mut framed = Framed::from_parts(parts);
    // 等待响应

    let status = framed.next().await.ok_or(ConnectError::Io(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "Failed to read status",
    )))??;

    match status.code() {
        v1::BridgeCode::Ok => {
            let FramedParts {
                io,
                read_buffer,
                write_buffer,
                ..
            } = framed.into_parts();
            assert!(
                write_buffer.is_empty(),
                "Expect a flushed Framed to have an empty write buffer."
            );
            Ok((io, read_buffer.freeze()))
        }
        code => Err(ConnectError::BridgeCode(code)),
    }
}

pub(crate) async fn make_bridge_relay_connect(
    io: Substream,
    src_peer_id: PeerId,
    dst_peer_id: PeerId,
    relayed_addr: Multiaddr,
) -> Result<(Substream, Bytes), ConnectError> {
    let mut dst_framed = Framed::new(
        io,
        ProtobufUviCodec::<v1::BridgeRelayConnect>::new(MAX_MESSAGE_SIZE),
    );
    let message = v1::BridgeRelayConnect {
        src_peer_id: src_peer_id.into_base58(),
        dst_peer_id: dst_peer_id.into_base58(),
        src_relayed_addr: relayed_addr.to_string(),
    };
    dst_framed.send(message).await?;
    dst_framed.flush().await?;

    let parts = dst_framed
        .into_parts()
        .map_codec(|_| ProtobufUviCodec::<v1::BridgeStatus>::new(MAX_MESSAGE_SIZE));

    let mut dst_framed = Framed::from_parts(parts);

    // 等待响应
    let status = dst_framed.next().await.ok_or(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "Failed to read status",
    ))??;
    match status.code() {
        v1::BridgeCode::Ok => {
            let FramedParts {
                io,
                read_buffer,
                write_buffer,
                ..
            } = dst_framed.into_parts();
            assert!(
                write_buffer.is_empty(),
                "Expect a flushed Framed to have an empty write buffer."
            );
            Ok((io, read_buffer.freeze()))
        }
        code => Err(ConnectError::BridgeCode(code)),
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConnectError {
    #[error("Bridge unsupported")]
    Unsupported,
    #[error("Invalid protocol")]
    BridgeCode(v1::BridgeCode),
    #[error("I/O error")]
    Io(#[from] io::Error),
}

// 处理一个桥接连接请求
pub(crate) async fn handle_bridge_connect(io: Substream) -> Result<Bridge, Error> {
    let mut framed = Framed::new(
        io,
        ProtobufUviCodec::<v1::BridgeConnect>::new(MAX_MESSAGE_SIZE),
    );

    let request = framed.next().await.ok_or(Error::Io(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "Failed to read request",
    )))??;

    let peer = request.peer.ok_or(ProtocolError::MissingPeer)?;

    let dst_peer_id = PeerId::try_from_base58(&peer.id).map_err(ProtocolError::from)?;
    let dst_addresses = peer
        .addresses
        .into_iter()
        .map(|r| Multiaddr::from_str(&r))
        .collect::<Result<Vec<_>, _>>()
        .map_err(ProtocolError::from)?;

    Ok(Bridge {
        circuit: Circuit::new(framed),
        dst_peer_id,
        dst_addresses,
    })
}

// 处理一个桥接中继连接请求
pub(crate) async fn handle_bridge_relay_connect(io: Substream) -> Result<Relay, Error> {
    let mut framed = Framed::new(
        io,
        ProtobufUviCodec::<v1::BridgeRelayConnect>::new(MAX_MESSAGE_SIZE),
    );
    let request = framed.next().await.ok_or(Error::Io(io::Error::new(
        io::ErrorKind::UnexpectedEof,
        "Failed to read relay request",
    )))??;

    let src_peer_id = PeerId::try_from_base58(&request.src_peer_id).map_err(ProtocolError::from)?;
    let dst_peer_id = PeerId::try_from_base58(&request.dst_peer_id).map_err(ProtocolError::from)?;
    let src_relayed_addr =
        Multiaddr::from_str(&request.src_relayed_addr).map_err(ProtocolError::from)?;
    let circuit = Circuit::new(framed);
    Ok(Relay {
        circuit,
        src_peer_id,
        dst_peer_id,
        src_relayed_addr,
    })
}

pub(crate) struct Circuit {
    framed: Framed<Substream, ProtobufUviCodec<v1::BridgeStatus>>,
}

impl Circuit {
    pub(crate) fn new<M>(framed: Framed<Substream, ProtobufUviCodec<M>>) -> Self
    where
        M: prost::Message + Default,
    {
        let parts = framed
            .into_parts()
            .map_codec(|_| ProtobufUviCodec::<v1::BridgeStatus>::new(MAX_MESSAGE_SIZE));

        let framed = Framed::from_parts(parts);

        Self { framed }
    }
}

pub(crate) struct Bridge {
    pub(crate) circuit: Circuit,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) dst_addresses: Vec<Multiaddr>,
}

impl Circuit {
    pub(crate) async fn accept(mut self) -> Result<(Substream, Bytes), io::Error> {
        self.send(v1::BridgeCode::Ok).await?;

        let FramedParts {
            io,
            read_buffer,
            write_buffer,
            ..
        } = self.framed.into_parts();
        assert!(
            write_buffer.is_empty(),
            "Expect a flushed Framed to have an empty write buffer."
        );

        Ok((io, read_buffer.freeze()))
    }

    pub(crate) async fn deny(mut self, code: v1::BridgeCode) -> Result<(), io::Error> {
        self.send(code).await?;
        Ok(())
    }

    async fn send(&mut self, code: v1::BridgeCode) -> Result<(), io::Error> {
        self.framed
            .send(v1::BridgeStatus { code: code as i32 })
            .await?;
        self.framed.flush().await?;
        Ok(())
    }
}

pub struct Relay {
    pub(crate) circuit: Circuit,
    pub(crate) src_peer_id: PeerId,
    pub(crate) dst_peer_id: PeerId,
    pub(crate) src_relayed_addr: Multiaddr,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("Invalid protocol")]
    Protocol(#[from] ProtocolError),
    #[error("I/O error")]
    Io(#[from] io::Error),
}

impl From<Error> for io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Protocol(e) => io::Error::new(io::ErrorKind::Other, e),
            Error::Io(e) => e,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ProtocolError {
    #[error("Expected 'peer' field to be set.")]
    MissingPeer,

    #[error(transparent)]
    InvalidPeerId(#[from] volans_core::identity::Error),
    #[error(transparent)]
    InvalidMultiaddr(#[from] volans_core::multiaddr::Error),
}
