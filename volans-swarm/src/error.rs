use std::{error, fmt, io};

use volans_core::{PeerId, TransportError, Url};

use crate::dial_opts;

#[derive(Debug, thiserror::Error)]
#[error("Connection denied: {inner}")]
pub struct ConnectionDenied {
    inner: Box<dyn std::error::Error + Send + Sync + 'static>,
}

impl ConnectionDenied {
    pub fn new(cause: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>) -> Self {
        Self {
            inner: cause.into(),
        }
    }

    pub fn downcast<E>(self) -> Result<E, Self>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let inner = self
            .inner
            .downcast::<E>()
            .map_err(|inner| ConnectionDenied { inner })?;

        Ok(*inner)
    }

    pub fn downcast_ref<E>(&self) -> Option<&E>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        self.inner.downcast_ref::<E>()
    }
}

/// 拨号错误
#[derive(Debug, thiserror::Error)]
pub enum DialError {
    LocalPeerId,
    NoAddress,
    PeerCondition(dial_opts::PeerCondition),
    Aborted,
    WrongPeerId {
        obtained: PeerId,
    },
    Denied {
        #[source]
        cause: ConnectionDenied,
    },
    Transport {
        addr: Url,
        #[source]
        error: TransportError<io::Error>,
    },
}

impl From<PendingConnectionError> for DialError {
    fn from(error: PendingConnectionError) -> Self {
        match error {
            PendingConnectionError::Transport { addr, error } => {
                DialError::Transport { addr, error }
            }
            PendingConnectionError::Aborted => DialError::Aborted,
            PendingConnectionError::WrongPeerId { obtained } => DialError::WrongPeerId { obtained },
            PendingConnectionError::LocalPeerId => DialError::LocalPeerId,
        }
    }
}

impl fmt::Display for DialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DialError::LocalPeerId => write!(f, "Local peer ID is not set"),
            DialError::NoAddress => write!(f, "No address to dial"),
            DialError::PeerCondition(condition) => write!(f, "Peer condition not met: {condition}"),
            DialError::Aborted => write!(f, "Dialing was aborted"),
            DialError::WrongPeerId { obtained } => {
                write!(f, "Dialed wrong peer ID: {obtained}")
            }
            DialError::Denied { cause } => write!(f, "Dialing denied: {cause}"),
            DialError::Transport { addr, error } => {
                write!(f, "Transport error while dialing `{addr}`, ")?;
                print_error_chain(f, error)
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ListenError {
    Aborted,
    WrongPeerId {
        obtained: PeerId,
    },
    LocalPeerId,
    Denied {
        #[source]
        cause: ConnectionDenied,
    },
    Transport(#[source] TransportError<io::Error>),
}

impl From<PendingConnectionError> for ListenError {
    fn from(error: PendingConnectionError) -> Self {
        match error {
            PendingConnectionError::Transport { addr: _, error } => {
                ListenError::Transport(TransportError::from(error))
            }
            PendingConnectionError::Aborted => ListenError::Aborted,
            PendingConnectionError::WrongPeerId { obtained } => {
                ListenError::WrongPeerId { obtained }
            }
            PendingConnectionError::LocalPeerId => ListenError::LocalPeerId,
        }
    }
}

impl fmt::Display for ListenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenError::Aborted => write!(f, "Listening was aborted"),
            ListenError::WrongPeerId { obtained } => {
                write!(f, "Listening on wrong peer ID: {obtained}")
            }
            ListenError::LocalPeerId => write!(f, "Local peer ID is not set"),
            ListenError::Denied { cause } => write!(f, "Listening denied: {cause}"),
            ListenError::Transport(error) => {
                write!(f, "Transport error while listening, ")?;
                print_error_chain(f, error)
            }
        }
    }
}

fn print_error_chain(f: &mut fmt::Formatter<'_>, e: &dyn error::Error) -> fmt::Result {
    write!(f, ": {e}")?;

    if let Some(source) = e.source() {
        print_error_chain(f, source)?;
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    #[error("Connection I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Connection keep-alive timeout")]
    KeepAliveTimeout,
    #[error("Connection closing")]
    Closing,
}

#[derive(Debug)]
pub enum PendingConnectionError {
    Transport {
        addr: Url,
        error: TransportError<io::Error>,
    },
    Aborted,
    WrongPeerId {
        obtained: PeerId,
    },
    LocalPeerId,
}
