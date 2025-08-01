pub mod inbound;
pub mod outbound;
mod protocol;

use std::time::Duration;

use volans_core::PeerId;
use volans_swarm::ConnectionId;

#[derive(Debug, Clone)]
pub struct Config {
    timeout: Duration,
    interval: Duration,
    failures: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(1),
            interval: Duration::from_secs(10),
            failures: 3,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Failure {
    #[error("Ping timeout")]
    Timeout,
    #[error("Ping protocol not supported")]
    Unsupported,
    #[error("Ping protocol error: {error}")]
    Other {
        error: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl Failure {
    fn other(e: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Other { error: Box::new(e) }
    }
}

#[derive(Debug)]
pub struct Event {
    pub connection: ConnectionId,
    pub peer_id: PeerId,
    pub result: Result<Duration, Failure>,
}
