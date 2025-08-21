mod mdns;

pub mod discovery;
pub mod registry;

use std::{
    collections::HashMap,
    task::{Context, Poll},
    time::Duration,
};

pub use mdns::{MdnsDiscovery, MdnsRegistry};

use volans_core::{Multiaddr, PeerId};

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    /// 服务名称
    pub name: String,
    /// 对等节点ID
    pub peer_id: PeerId,
    /// 监听地址列表
    pub addresses: Vec<Multiaddr>,
    /// 服务元数据
    pub metadata: HashMap<String, String>,
    /// 服务的TTL（生存时间）
    pub ttl: Duration,
}

pub trait Registry: Default + Send + 'static {
    type Discovery: Discovery;
    fn register(&mut self, service: ServiceInfo) -> Result<(), RegistryError>;
    fn deregister(&mut self, peer_id: PeerId) -> Result<(), RegistryError>;

    fn discovery(&self) -> Result<Self::Discovery, RegistryError>;

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Result<RegisterEvent, RegistryError>>;
}

pub trait Discovery: Send + 'static {
    fn poll_watch(&mut self, cx: &mut Context<'_>) -> Poll<Result<DiscoveryEvent, RegistryError>>;
}

pub enum RegisterEvent {
    Registered(ServiceInfo),
    Deregistered(PeerId),
}

pub enum DiscoveryEvent {
    Discovered(ServiceInfo),
    Expired(ServiceInfo),
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Service not found")]
    ServiceNotFound,
    #[error("Peer ID not found in service info")]
    PeerIdNotFound,
    #[error("Invalid Peer ID: {0}")]
    InvalidPeerId(#[from] volans_core::identity::Error),
    #[error("Invalid Multiaddr: {0}")]
    InvalidMultiaddr(#[from] volans_core::multiaddr::Error),
    #[error("Registry closed")]
    Closed,
    #[error("Registry error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

pub struct Config {
    /// 服务名称
    pub name: String,
    /// 服务的元数据
    pub metadata: HashMap<String, String>,
    /// 服务的TTL（生存时间）
    pub ttl: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "volans".to_string(),
            metadata: HashMap::new(),
            ttl: Duration::from_secs(60), // 默认TTL为60秒
        }
    }
}
