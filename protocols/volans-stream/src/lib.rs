mod upgrade;

pub mod client;
pub mod server;

use std::fmt;

pub use upgrade::{InboundStreamUpgradeFactory, OutboundStreamUpgradeFactory, ReadyUpgrade};
use volans_swarm::StreamProtocol;

pub enum StreamEvent<TOutput, TError> {
    FullyNegotiated {
        protocol: StreamProtocol,
        output: TOutput,
    },
    UpgradeError {
        error: TError,
    },
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
            StreamEvent::UpgradeError { error } => f
                .debug_struct("StreamEvent::UpgradeError")
                .field("error", error)
                .finish(),
        }
    }
}
