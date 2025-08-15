use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

use crate::PeerId;

mod error;
mod from_url;
mod protocol;

pub use error::Error;
pub use from_url::{FromUrlErr, from_url, from_url_lossy};
pub use protocol::Protocol;

#[allow(clippy::rc_buffer)]
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct Multiaddr {
    bytes: Bytes,
}

impl Multiaddr {
    pub fn empty() -> Self {
        Self {
            bytes: Bytes::new(),
        }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            bytes: BytesMut::with_capacity(n).freeze(),
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.len() == 0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        Vec::from(&self.bytes[..])
    }

    pub fn push(&mut self, p: Protocol<'_>) {
        let mut bytes = BytesMut::from(std::mem::take(&mut self.bytes));
        p.write_bytes(&mut (&mut bytes).writer())
            .expect("Writing to a `BytesMut` never fails.");
        self.bytes = bytes.freeze();
    }

    pub fn pop<'a>(&mut self) -> Option<Protocol<'a>> {
        let mut slice = &self.bytes[..]; // the remaining multiaddr slice
        if slice.is_empty() {
            return None;
        }
        let protocol = loop {
            let (p, s) = Protocol::from_bytes(slice).expect("`slice` is a valid `Protocol`.");
            if s.is_empty() {
                break p.acquire();
            }
            slice = s
        };
        let remaining_len = self.len() - slice.len();
        let mut bytes = BytesMut::from(std::mem::take(&mut self.bytes));
        bytes.truncate(remaining_len);
        self.bytes = bytes.freeze();
        Some(protocol)
    }

    pub fn with(mut self, p: Protocol<'_>) -> Self {
        let mut bytes = BytesMut::from(std::mem::take(&mut self.bytes));
        p.write_bytes(&mut (&mut bytes).writer())
            .expect("Writing to a `BytesMut` never fails.");
        self.bytes = bytes.freeze();
        self
    }

    pub fn with_peer(self, peer: PeerId) -> std::result::Result<Self, Self> {
        match self.iter().last() {
            Some(Protocol::Peer(p)) if p == peer => Ok(self),
            Some(Protocol::Peer(_)) => Err(self),
            _ => Ok(self.with(Protocol::Peer(peer))),
        }
    }

    pub fn iter(&self) -> Iter<'_> {
        Iter(&self.bytes)
    }

    pub fn replace<'a, F>(&self, at: usize, by: F) -> Option<Multiaddr>
    where
        F: FnOnce(&Protocol<'_>) -> Option<Protocol<'a>>,
    {
        let mut address = Multiaddr::with_capacity(self.len());
        let mut fun = Some(by);
        let mut replaced = false;

        for (i, p) in self.iter().enumerate() {
            if i == at {
                let f = fun.take().expect("i == at only happens once");
                if let Some(q) = f(&p) {
                    address = address.with(q);
                    replaced = true;
                    continue;
                }
                return None;
            }
            address = address.with(p)
        }

        if replaced { Some(address) } else { None }
    }

    pub fn ends_with(&self, other: &Multiaddr) -> bool {
        let n = self.bytes.len();
        let m = other.bytes.len();
        if n < m {
            return false;
        }
        self.bytes[(n - m)..] == other.bytes[..]
    }

    pub fn starts_with(&self, other: &Multiaddr) -> bool {
        let n = self.bytes.len();
        let m = other.bytes.len();
        if n < m {
            return false;
        }
        self.bytes[..m] == other.bytes[..]
    }

    pub fn protocol_stack(&self) -> ProtoStackIter {
        ProtoStackIter { parts: self.iter() }
    }
}

impl fmt::Debug for Multiaddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Multiaddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in self.iter() {
            s.fmt(f)?;
        }
        Ok(())
    }
}

impl AsRef<[u8]> for Multiaddr {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl<'a> IntoIterator for &'a Multiaddr {
    type Item = Protocol<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        Iter(&self.bytes)
    }
}

impl<'a> FromIterator<Protocol<'a>> for Multiaddr {
    fn from_iter<T>(iter: T) -> Self
    where
        T: IntoIterator<Item = Protocol<'a>>,
    {
        let mut bytes = BytesMut::new();
        for cmp in iter {
            cmp.write_bytes(&mut (&mut bytes).writer())
                .expect("Writing to a `BytesMut` never fails.");
        }
        Multiaddr {
            bytes: bytes.freeze(),
        }
    }
}

impl FromStr for Multiaddr {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Error> {
        let mut bytes = BytesMut::new();
        let mut parts = input.split('/').peekable();

        if Some("") != parts.next() {
            // A multiaddr must start with `/`
            return Err(Error::InvalidMultiaddr);
        }

        while parts.peek().is_some() {
            let p = Protocol::from_str_parts(&mut parts)?;
            p.write_bytes(&mut (&mut bytes).writer())
                .expect("Writing to a `BytesMut` never fails.");
        }

        Ok(Multiaddr {
            bytes: bytes.freeze(),
        })
    }
}

/// Iterator over `Multiaddr` [`Protocol`]s.
pub struct Iter<'a>(&'a [u8]);

impl<'a> Iterator for Iter<'a> {
    type Item = Protocol<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }

        let (p, next_data) =
            Protocol::from_bytes(self.0).expect("`Multiaddr` is known to be valid.");

        self.0 = next_data;
        Some(p)
    }
}

/// Iterator over the string identifiers of the protocols (not addrs) in a multiaddr
pub struct ProtoStackIter<'a> {
    parts: Iter<'a>,
}

impl Iterator for ProtoStackIter<'_> {
    type Item = &'static str;
    fn next(&mut self) -> Option<Self::Item> {
        self.parts.next().as_ref().map(Protocol::tag)
    }
}

impl<'a> From<Protocol<'a>> for Multiaddr {
    fn from(p: Protocol<'a>) -> Multiaddr {
        let mut bytes = BytesMut::new();
        p.write_bytes(&mut (&mut bytes).writer())
            .expect("Writing to a `BytesMut` never fails.");
        Multiaddr {
            bytes: bytes.freeze(),
        }
    }
}

impl From<IpAddr> for Multiaddr {
    fn from(v: IpAddr) -> Multiaddr {
        match v {
            IpAddr::V4(a) => a.into(),
            IpAddr::V6(a) => a.into(),
        }
    }
}

impl From<Ipv4Addr> for Multiaddr {
    fn from(v: Ipv4Addr) -> Multiaddr {
        Protocol::Ip4(v).into()
    }
}

impl From<Ipv6Addr> for Multiaddr {
    fn from(v: Ipv6Addr) -> Multiaddr {
        Protocol::Ip6(v).into()
    }
}

impl TryFrom<Vec<u8>> for Multiaddr {
    type Error = Error;

    fn try_from(v: Vec<u8>) -> Result<Self, Error> {
        // Check if the argument is a valid `Multiaddr` by reading its protocols.
        let mut slice = &v[..];
        while !slice.is_empty() {
            let (_, s) = Protocol::from_bytes(slice)?;
            slice = s
        }
        Ok(Multiaddr {
            bytes: Bytes::from(v),
        })
    }
}

impl TryFrom<String> for Multiaddr {
    type Error = Error;

    fn try_from(s: String) -> Result<Multiaddr, Error> {
        s.parse()
    }
}

impl<'a> TryFrom<&'a str> for Multiaddr {
    type Error = Error;

    fn try_from(s: &'a str) -> Result<Multiaddr, Error> {
        s.parse()
    }
}

impl Serialize for Multiaddr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_string())
        } else {
            serializer.serialize_bytes(self.as_ref())
        }
    }
}

impl<'de> Deserialize<'de> for Multiaddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor {
            is_human_readable: bool,
        }

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = Multiaddr;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("multiaddress")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut buf: Vec<u8> =
                    Vec::with_capacity(std::cmp::min(seq.size_hint().unwrap_or(0), 4096));
                while let Some(e) = seq.next_element()? {
                    buf.push(e);
                }
                if self.is_human_readable {
                    let s = String::from_utf8(buf).map_err(de::Error::custom)?;
                    s.parse().map_err(de::Error::custom)
                } else {
                    Multiaddr::try_from(buf).map_err(de::Error::custom)
                }
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                v.parse().map_err(de::Error::custom)
            }
            fn visit_borrowed_str<E: de::Error>(self, v: &'de str) -> Result<Self::Value, E> {
                self.visit_str(v)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
                self.visit_byte_buf(v.into())
            }
            fn visit_borrowed_bytes<E: de::Error>(self, v: &'de [u8]) -> Result<Self::Value, E> {
                self.visit_byte_buf(v.into())
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
                Multiaddr::try_from(v).map_err(de::Error::custom)
            }
        }

        if deserializer.is_human_readable() {
            deserializer.deserialize_str(Visitor {
                is_human_readable: true,
            })
        } else {
            deserializer.deserialize_bytes(Visitor {
                is_human_readable: false,
            })
        }
    }
}
