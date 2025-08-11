mod upgrade;

pub mod client;
pub mod server;

use std::fmt;

pub use upgrade::{InboundStreamUpgradeFactory, OutboundStreamUpgradeFactory, ReadyUpgrade};
use volans_core::PeerId;
use volans_swarm::{ConnectionId, StreamProtocol};

pub enum StreamEvent<TOutput, TError> {
    FullyNegotiated {
        protocol: StreamProtocol,
        output: TOutput,
    },
    UpgradeError {
        protocol: StreamProtocol,
        error: TError,
    },
}

impl<TOutput, TError> StreamEvent<TOutput, TError> {
    pub fn protocol(&self) -> StreamProtocol {
        match self {
            StreamEvent::FullyNegotiated { protocol, .. } => protocol.clone(),
            StreamEvent::UpgradeError { protocol, .. } => protocol.clone(),
        }
    }
}

impl<TOutput, TError> fmt::Debug for StreamEvent<TOutput, TError>
where
    TError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamEvent::FullyNegotiated {
                output: _,
                protocol,
            } => f
                .debug_struct("StreamEvent::FullyNegotiated")
                .field("protocol", protocol)
                .finish(),
            StreamEvent::UpgradeError { protocol, error } => f
                .debug_struct("StreamEvent::UpgradeError")
                .field("protocol", protocol)
                .field("error", error)
                .finish(),
        }
    }
}

pub enum ConnectionStreamEvent<TOutput, TError> {
    FullyNegotiated {
        peer_id: PeerId,
        connection_id: ConnectionId,
        protocol: StreamProtocol,
        output: TOutput,
    },
    UpgradeError {
        peer_id: PeerId,
        connection_id: ConnectionId,
        protocol: StreamProtocol,
        error: TError,
    },
}

impl<TOutput, TError> ConnectionStreamEvent<TOutput, TError>
where
    TError: fmt::Debug,
{
    pub fn peer_id(&self) -> PeerId {
        match self {
            ConnectionStreamEvent::FullyNegotiated { peer_id, .. } => *peer_id,
            ConnectionStreamEvent::UpgradeError { peer_id, .. } => *peer_id,
        }
    }

    pub fn connection_id(&self) -> ConnectionId {
        match self {
            ConnectionStreamEvent::FullyNegotiated { connection_id, .. } => *connection_id,
            ConnectionStreamEvent::UpgradeError { connection_id, .. } => *connection_id,
        }
    }

    pub fn protocol(&self) -> StreamProtocol {
        match self {
            ConnectionStreamEvent::FullyNegotiated { protocol, .. } => protocol.clone(),
            ConnectionStreamEvent::UpgradeError { protocol, .. } => protocol.clone(),
        }
    }
}

impl<TOutput, TError> fmt::Debug for ConnectionStreamEvent<TOutput, TError>
where
    TError: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectionStreamEvent::FullyNegotiated {
                peer_id,
                connection_id,
                protocol,
                ..
            } => f
                .debug_struct("IncomingEvent::FullyNegotiated")
                .field("peer_id", peer_id)
                .field("connection_id", connection_id)
                .field("protocol", protocol)
                .finish(),
            ConnectionStreamEvent::UpgradeError {
                peer_id,
                connection_id,
                protocol,
                error,
            } => f
                .debug_struct("IncomingEvent::UpgradeError")
                .field("peer_id", peer_id)
                .field("connection_id", connection_id)
                .field("protocol", protocol)
                .field("error", error)
                .finish(),
        }
    }
}
