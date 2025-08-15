use std::{fmt, str::FromStr};

pub use ed25519_dalek::{
    SecretKey, SignatureError, SigningKey as KeyPair, VerifyingKey as PublicKey,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerId([u8; 32]);

impl PeerId {
    pub fn from_public_key(key: &PublicKey) -> Self {
        Self(key.to_bytes())
    }

    pub fn random() -> Self {
        Self(rand::random())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() == 32 {
            let mut array = [0u8; 32];
            array.copy_from_slice(bytes);
            Ok(Self(array))
        } else {
            Err(Error::LengthInvalid)
        }
    }

    pub fn into_bytes(&self) -> [u8; 32] {
        self.0
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn into_base58(self) -> String {
        bs58::encode(self.into_bytes()).into_string()
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PeerId").field(&self.into_base58()).finish()
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.into_base58().fmt(f)
    }
}

impl Serialize for PeerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.into_base58())
        } else {
            serializer.serialize_bytes(&self.into_bytes()[..])
        }
    }
}

impl<'de> Deserialize<'de> for PeerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::*;

        struct PeerIdVisitor;

        impl Visitor<'_> for PeerIdVisitor {
            type Value = PeerId;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "valid peer id")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: Error,
            {
                PeerId::try_from_slice(v)
                    .map_err(|_| Error::invalid_value(Unexpected::Bytes(v), &self))
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                PeerId::from_str(v).map_err(|_| Error::invalid_value(Unexpected::Str(v), &self))
            }
        }

        if deserializer.is_human_readable() {
            deserializer.deserialize_str(PeerIdVisitor)
        } else {
            deserializer.deserialize_bytes(PeerIdVisitor)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base-58 decode error: {0}")]
    Bs58(#[from] bs58::decode::Error),
    #[error("PeerId length invalid, expected 32 bytes")]
    LengthInvalid,
}

impl FromStr for PeerId {
    type Err = Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = bs58::decode(s).into_vec()?;
        let peer_id = PeerId::try_from_slice(&bytes)?;
        Ok(peer_id)
    }
}
