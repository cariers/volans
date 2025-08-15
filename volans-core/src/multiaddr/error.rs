use std::{io, net, num, str, string};

use unsigned_varint::decode;

use crate::identity;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("Data length is less than expected")]
    DataLessThanLen,
    #[error("Invalid multiaddr protocol")]
    InvalidMultiaddr,
    #[error("Invalid protocol string")]
    InvalidProtocol,
    #[error("Invalid varint: {0}")]
    InvalidVarint(#[from] decode::Error),
    #[error("Failed to parse: {0}")]
    ParsingError(Box<dyn std::error::Error + Send + Sync>),
    #[error("Unknown protocol ID: {0}")]
    UnknownProtocolId(u32),
    #[error("Unknown protocol: {0}")]
    UnknownProtocol(String),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::ParsingError(err.into())
    }
}

impl From<net::AddrParseError> for Error {
    fn from(err: net::AddrParseError) -> Error {
        Error::ParsingError(err.into())
    }
}

impl From<num::ParseIntError> for Error {
    fn from(err: num::ParseIntError) -> Error {
        Error::ParsingError(err.into())
    }
}

impl From<string::FromUtf8Error> for Error {
    fn from(err: string::FromUtf8Error) -> Error {
        Error::ParsingError(err.into())
    }
}

impl From<str::Utf8Error> for Error {
    fn from(err: str::Utf8Error) -> Error {
        Error::ParsingError(err.into())
    }
}

impl From<identity::Error> for Error {
    fn from(err: identity::Error) -> Error {
        Error::ParsingError(err.into())
    }
}
