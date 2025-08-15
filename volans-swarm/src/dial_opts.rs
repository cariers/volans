use std::fmt;

use volans_core::{PeerId, Multiaddr};

use crate::ConnectionId;

#[derive(Debug)]
pub struct DialOpts {
    peer_id: Option<PeerId>,
    condition: PeerCondition,
    addr: Option<Multiaddr>,
    connection_id: ConnectionId,
}

impl DialOpts {
    pub fn new(addr: Option<Multiaddr>, peer_id: Option<PeerId>) -> Self {
        Self {
            peer_id,
            condition: PeerCondition::default(),
            addr,
            connection_id: ConnectionId::next(),
        }
    }

    pub fn with_condition(mut self, condition: PeerCondition) -> Self {
        self.condition = condition;
        self
    }

    pub fn peer_id(&self) -> Option<PeerId> {
        self.peer_id
    }

    pub fn condition(&self) -> PeerCondition {
        self.condition
    }

    pub fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    pub fn addr(&self) -> Option<Multiaddr> {
        self.addr.clone()
    }
}

#[derive(Debug, Copy, Clone, Default)]
pub enum PeerCondition {
    /// 总是建立新连接
    #[default]
    Always,
    /// 没有可用连接
    Disconnected,
    /// 没有正在拨号时
    NotDialing,
    /// 断开连接且没有正在拨号时
    DisconnectedAndNotDialing,
}

impl fmt::Display for PeerCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerCondition::Always => write!(f, "always"),
            PeerCondition::Disconnected => write!(f, "disconnected"),
            PeerCondition::NotDialing => write!(f, "not dialing"),
            PeerCondition::DisconnectedAndNotDialing => write!(f, "disconnected and not dialing"),
        }
    }
}
