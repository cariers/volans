use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::{
    borrow::Cow,
    convert::From,
    fmt,
    io::{Cursor, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};
use unsigned_varint::{decode, encode};

use crate::{PeerId, multiaddr::Error};

const DNS: u32 = 53;
const DNS4: u32 = 54;
const DNS6: u32 = 55;
const HTTP: u32 = 480;
const IP4: u32 = 4;
const IP6: u32 = 41;
const MEMORY: u32 = 777;
const PEER: u32 = 421;
const CIRCUIT: u32 = 290;
const QUIC: u32 = 460;
const TCP: u32 = 6;
const TLS: u32 = 448;
const UDP: u32 = 273;
const UNIX: u32 = 400;
const WS: u32 = 477;
const SNI: u32 = 449;
const PATH: u32 = 481;

const PATH_SEGMENT_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b'%')
    .add(b'/')
    .add(b'`')
    .add(b'?')
    .add(b'{')
    .add(b'}')
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b':');

#[derive(PartialEq, Eq, Clone, Debug)]
#[non_exhaustive]
pub enum Protocol<'a> {
    Dns(Cow<'a, str>),
    Dns4(Cow<'a, str>),
    Dns6(Cow<'a, str>),

    Ip4(Ipv4Addr),
    Ip6(Ipv6Addr),
    Unix,

    Memory(u64),
    Tcp(u16),
    Udp(u16),

    Tls,
    Http,
    Ws,
    Quic,

    Peer(PeerId),
    Circuit,

    Sni(Cow<'a, str>),
    Path(Cow<'a, str>),
}

impl<'a> Protocol<'a> {
    pub fn from_str_parts<I>(mut iter: I) -> Result<Self, Error>
    where
        I: Iterator<Item = &'a str>,
    {
        match iter.next().ok_or(Error::InvalidProtocol)? {
            "dns" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                Ok(Protocol::Dns(Cow::Borrowed(s)))
            }
            "dns4" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                Ok(Protocol::Dns4(Cow::Borrowed(s)))
            }
            "dns6" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                Ok(Protocol::Dns6(Cow::Borrowed(s)))
            }
            "ip4" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let addr = Ipv4Addr::from_str(s)?;
                Ok(Protocol::Ip4(addr))
            }
            "ip6" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let addr = Ipv6Addr::from_str(s)?;
                Ok(Protocol::Ip6(addr))
            }
            "unix" => Ok(Protocol::Unix),
            "memory" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let port = s.parse::<u64>()?;
                Ok(Protocol::Memory(port))
            }
            "tcp" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let port = s.parse::<u16>()?;
                Ok(Protocol::Tcp(port))
            }
            "udp" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let port = s.parse::<u16>()?;
                Ok(Protocol::Udp(port))
            }
            "tls" => Ok(Protocol::Tls),
            "http" => Ok(Protocol::Http),
            "ws" => Ok(Protocol::Ws),
            "quic" => Ok(Protocol::Quic),
            "peer" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                Ok(Protocol::Peer(PeerId::from_str(s)?))
            }
            "circuit" => Ok(Protocol::Circuit),
            "sni" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                Ok(Protocol::Sni(Cow::Borrowed(s)))
            }
            "x-with-path" => {
                let s = iter.next().ok_or(Error::InvalidProtocol)?;
                let decoded = percent_encoding::percent_decode(s.as_bytes()).decode_utf8()?;
                Ok(Protocol::Path(decoded))
            }
            unknown => Err(Error::UnknownProtocol(unknown.to_string())),
        }
    }

    pub fn from_bytes(input: &'a [u8]) -> Result<(Self, &'a [u8]), Error> {
        fn split_at(n: usize, input: &[u8]) -> Result<(&[u8], &[u8]), Error> {
            if input.len() < n {
                return Err(Error::DataLessThanLen);
            }
            Ok(input.split_at(n))
        }
        let (id, input) = decode::u32(input)?;
        match id {
            DNS => {
                let (n, input) = decode::usize(input)?;
                let (data, rest) = split_at(n, input)?;
                Ok((Protocol::Dns(Cow::Borrowed(str::from_utf8(data)?)), rest))
            }
            DNS4 => {
                let (n, input) = decode::usize(input)?;
                let (data, rest) = split_at(n, input)?;
                Ok((Protocol::Dns4(Cow::Borrowed(str::from_utf8(data)?)), rest))
            }
            DNS6 => {
                let (n, input) = decode::usize(input)?;
                let (data, rest) = split_at(n, input)?;
                Ok((Protocol::Dns6(Cow::Borrowed(str::from_utf8(data)?)), rest))
            }

            IP4 => {
                let (data, rest) = split_at(4, input)?;
                Ok((
                    Protocol::Ip4(Ipv4Addr::new(data[0], data[1], data[2], data[3])),
                    rest,
                ))
            }
            IP6 => {
                let (data, rest) = split_at(16, input)?;
                let mut rdr = Cursor::new(data);
                let mut seg = [0_u16; 8];

                for x in seg.iter_mut() {
                    *x = rdr.read_u16::<BigEndian>()?;
                }

                let addr = Ipv6Addr::new(
                    seg[0], seg[1], seg[2], seg[3], seg[4], seg[5], seg[6], seg[7],
                );

                Ok((Protocol::Ip6(addr), rest))
            }
            UNIX => Ok((Protocol::Unix, input)),
            MEMORY => {
                let (data, rest) = split_at(8, input)?;
                let mut rdr = Cursor::new(data);
                let num = rdr.read_u64::<BigEndian>()?;
                Ok((Protocol::Memory(num), rest))
            }
            TCP => {
                let (data, rest) = split_at(2, input)?;
                let mut rdr = Cursor::new(data);
                let num = rdr.read_u16::<BigEndian>()?;
                Ok((Protocol::Tcp(num), rest))
            }
            UDP => {
                let (data, rest) = split_at(2, input)?;
                let mut rdr = Cursor::new(data);
                let num = rdr.read_u16::<BigEndian>()?;
                Ok((Protocol::Udp(num), rest))
            }
            TLS => Ok((Protocol::Tls, input)),
            HTTP => Ok((Protocol::Http, input)),
            WS => Ok((Protocol::Ws, input)),
            QUIC => Ok((Protocol::Quic, input)),
            PEER => {
                let (data, rest) = split_at(32, input)?;
                let peer_id = PeerId::try_from_slice(data)?;
                Ok((Protocol::Peer(peer_id), rest))
            }
            CIRCUIT => Ok((Protocol::Circuit, input)),
            SNI => {
                let (n, input) = decode::usize(input)?;
                let (data, rest) = split_at(n, input)?;
                Ok((Protocol::Sni(Cow::Borrowed(str::from_utf8(data)?)), rest))
            }
            PATH => {
                let (n, input) = decode::usize(input)?;
                let (data, rest) = split_at(n, input)?;
                Ok((Protocol::Path(Cow::Borrowed(str::from_utf8(data)?)), rest))
            }
            _ => Err(Error::UnknownProtocolId(id)),
        }
    }

    pub fn write_bytes<W: Write>(&self, w: &mut W) -> Result<(), Error> {
        let mut buf = encode::u32_buffer();
        match self {
            Protocol::Dns(cow) => {
                w.write_all(encode::u32(DNS, &mut buf))?;
                let bytes = cow.as_bytes();
                w.write_all(encode::usize(bytes.len(), &mut encode::usize_buffer()))?;
                w.write_all(bytes)?
            }
            Protocol::Dns4(cow) => {
                w.write_all(encode::u32(DNS4, &mut buf))?;
                let bytes = cow.as_bytes();
                w.write_all(encode::usize(bytes.len(), &mut encode::usize_buffer()))?;
                w.write_all(bytes)?
            }
            Protocol::Dns6(cow) => {
                w.write_all(encode::u32(DNS6, &mut buf))?;
                let bytes = cow.as_bytes();
                w.write_all(encode::usize(bytes.len(), &mut encode::usize_buffer()))?;
                w.write_all(bytes)?
            }

            Protocol::Ip4(addr) => {
                w.write_all(encode::u32(IP4, &mut buf))?;
                w.write_all(&addr.octets())?
            }
            Protocol::Ip6(addr) => {
                w.write_all(encode::u32(IP6, &mut buf))?;
                for &segment in &addr.segments() {
                    w.write_u16::<BigEndian>(segment)?
                }
            }
            Protocol::Unix => {
                w.write_all(encode::u32(UNIX, &mut buf))?;
            }

            Protocol::Memory(port) => {
                w.write_all(encode::u32(MEMORY, &mut buf))?;
                w.write_u64::<BigEndian>(*port)?
            }
            Protocol::Tcp(port) => {
                w.write_all(encode::u32(TCP, &mut buf))?;
                w.write_u16::<BigEndian>(*port)?
            }
            Protocol::Udp(port) => {
                w.write_all(encode::u32(UDP, &mut buf))?;
                w.write_u16::<BigEndian>(*port)?
            }

            Protocol::Tls => {
                w.write_all(encode::u32(TLS, &mut buf))?;
            }
            Protocol::Http => w.write_all(encode::u32(HTTP, &mut buf))?,
            Protocol::Ws => {
                w.write_all(encode::u32(WS, &mut buf))?;
            }
            Protocol::Quic => {
                w.write_all(encode::u32(QUIC, &mut buf))?;
            }

            Protocol::Peer(p) => {
                w.write_all(encode::u32(PEER, &mut buf))?;
                w.write_all(p.as_bytes())?
            }
            Protocol::Circuit => {
                w.write_all(encode::u32(CIRCUIT, &mut buf))?;
            }

            Protocol::Sni(cow) => {
                w.write_all(encode::u32(SNI, &mut buf))?;
                let bytes = cow.as_bytes();
                w.write_all(encode::usize(bytes.len(), &mut encode::usize_buffer()))?;
                w.write_all(bytes)?
            }
            Protocol::Path(cow) => {
                w.write_all(encode::u32(PATH, &mut buf))?;
                let bytes = cow.as_bytes();
                w.write_all(encode::usize(bytes.len(), &mut encode::usize_buffer()))?;
                w.write_all(bytes)?
            }
        }
        Ok(())
    }

    pub fn acquire<'b>(self) -> Protocol<'b> {
        match self {
            Protocol::Dns(cow) => Protocol::Dns(Cow::Owned(cow.into_owned())),
            Protocol::Dns4(cow) => Protocol::Dns4(Cow::Owned(cow.into_owned())),
            Protocol::Dns6(cow) => Protocol::Dns6(Cow::Owned(cow.into_owned())),
            Protocol::Http => Protocol::Http,
            Protocol::Ip4(a) => Protocol::Ip4(a),
            Protocol::Ip6(a) => Protocol::Ip6(a),
            Protocol::Memory(a) => Protocol::Memory(a),
            Protocol::Peer(a) => Protocol::Peer(a),
            Protocol::Circuit => Protocol::Circuit,
            Protocol::Quic => Protocol::Quic,
            Protocol::Tcp(a) => Protocol::Tcp(a),
            Protocol::Tls => Protocol::Tls,
            Protocol::Udp(a) => Protocol::Udp(a),
            Protocol::Unix => Protocol::Unix,
            Protocol::Ws => Protocol::Ws,
            Protocol::Sni(cow) => Protocol::Sni(Cow::Owned(cow.into_owned())),
            Protocol::Path(cow) => Protocol::Path(Cow::Owned(cow.into_owned())),
        }
    }

    pub fn tag(&self) -> &'static str {
        match self {
            Protocol::Dns(_) => "dns",
            Protocol::Dns4(_) => "dns4",
            Protocol::Dns6(_) => "dns6",
            Protocol::Http => "http",
            Protocol::Ip4(_) => "ip4",
            Protocol::Ip6(_) => "ip6",
            Protocol::Memory(_) => "memory",
            Protocol::Peer(_) => "peer",
            Protocol::Circuit => "circuit",
            Protocol::Quic => "quic",
            Protocol::Tcp(_) => "tcp",
            Protocol::Tls => "tls",
            Protocol::Udp(_) => "udp",
            Protocol::Unix => "unix",
            Protocol::Ws => "ws",
            Protocol::Sni(_) => "sni",
            Protocol::Path(_) => "x-with-path",
        }
    }
}

impl fmt::Display for Protocol<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}", self.tag())?;
        match self {
            Protocol::Dns(s) => write!(f, "/{s}"),
            Protocol::Dns4(s) => write!(f, "/{s}"),
            Protocol::Dns6(s) => write!(f, "/{s}"),
            Protocol::Ip4(addr) => write!(f, "/{addr}"),
            Protocol::Ip6(addr) => write!(f, "/{addr}"),
            Protocol::Memory(port) => write!(f, "/{port}"),
            Protocol::Peer(p) => write!(f, "/{p}"),
            Protocol::Tcp(port) => write!(f, "/{port}"),
            Protocol::Udp(port) => write!(f, "/{port}"),
            Protocol::Sni(s) => write!(f, "/{s}"),
            Protocol::Path(s) => {
                let encoded =
                    percent_encoding::percent_encode(s.as_bytes(), PATH_SEGMENT_ENCODE_SET);
                write!(f, "/{encoded}")
            }
            _ => Ok(()),
        }
    }
}

impl From<IpAddr> for Protocol<'_> {
    #[inline]
    fn from(addr: IpAddr) -> Self {
        match addr {
            IpAddr::V4(addr) => Protocol::Ip4(addr),
            IpAddr::V6(addr) => Protocol::Ip6(addr),
        }
    }
}

impl From<Ipv4Addr> for Protocol<'_> {
    #[inline]
    fn from(addr: Ipv4Addr) -> Self {
        Protocol::Ip4(addr)
    }
}

impl From<Ipv6Addr> for Protocol<'_> {
    #[inline]
    fn from(addr: Ipv6Addr) -> Self {
        Protocol::Ip6(addr)
    }
}
