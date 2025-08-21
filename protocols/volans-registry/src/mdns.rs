use std::{
    collections::{HashMap, VecDeque},
    task::{Context, Poll},
    time::Duration,
};

use flume::r#async::RecvStream;
use futures::StreamExt;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use volans_core::{Multiaddr, PeerId};

use crate::{Discovery, DiscoveryEvent, RegisterEvent, Registry, RegistryError, ServiceInfo};

const PROPERTY_PEER_ID: &str = "PEER_ID";
const PROPERTY_ADDR_PREFIX: &str = "DNS_ADDR_";

const SERVICE_NAME_FQDN: &str = "_volans._udp.local.";

pub struct MdnsRegistry {
    daemon: ServiceDaemon,
    registered_services: HashMap<PeerId, ServiceInfo>,
    pending_events: VecDeque<RegisterEvent>,
}

impl MdnsRegistry {
    pub fn new() -> Result<Self, RegistryError> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            registered_services: HashMap::new(),
            pending_events: VecDeque::new(),
        })
    }
}

impl Default for MdnsRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create MdnsRegistry")
    }
}

impl Registry for MdnsRegistry {
    type Discovery = MdnsDiscovery;

    fn register(&mut self, service: ServiceInfo) -> Result<(), RegistryError> {
        let inner_info = mdns_sd::ServiceInfo::try_from(service.clone())?;
        self.daemon.register(inner_info)?;
        self.registered_services
            .insert(service.peer_id, service.clone());
        self.pending_events
            .push_back(RegisterEvent::Registered(service));
        Ok(())
    }

    fn deregister(&mut self, peer_id: PeerId) -> Result<(), RegistryError> {
        let service_info = self
            .registered_services
            .remove(&peer_id)
            .ok_or(RegistryError::ServiceNotFound)?;
        let inner_info = mdns_sd::ServiceInfo::try_from(service_info)?;
        self.daemon.unregister(inner_info.get_fullname())?;
        self.pending_events
            .push_back(RegisterEvent::Deregistered(peer_id));
        Ok(())
    }

    fn discovery(&self) -> Result<Self::Discovery, RegistryError> {
        let receiver = self.daemon.browse(SERVICE_NAME_FQDN)?.into_stream();
        Ok(MdnsDiscovery {
            stream: receiver,
            discovered: HashMap::new(),
            fullname_map: HashMap::new(),
        })
    }

    fn poll_next(&mut self, _cx: &mut Context<'_>) -> Poll<Result<RegisterEvent, RegistryError>> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(Ok(event));
            }
            return Poll::Pending;
        }
    }
}

pub struct MdnsDiscovery {
    stream: RecvStream<'static, ServiceEvent>,
    discovered: HashMap<PeerId, ServiceInfo>,
    fullname_map: HashMap<String, PeerId>,
}

impl Discovery for MdnsDiscovery {
    fn poll_watch(&mut self, cx: &mut Context<'_>) -> Poll<Result<DiscoveryEvent, RegistryError>> {
        loop {
            match self.stream.poll_next_unpin(cx) {
                Poll::Ready(Some(event)) => match event {
                    ServiceEvent::ServiceResolved(info) => {
                        tracing::trace!("mdns watch resolved: {:?}", info);
                        let fullname = info.get_fullname().to_string();
                        let service_info = ServiceInfo::try_from(info)?;
                        self.discovered
                            .insert(service_info.peer_id, service_info.clone());
                        self.fullname_map.insert(fullname, service_info.peer_id);
                        return Poll::Ready(Ok(DiscoveryEvent::Discovered(service_info)));
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        tracing::trace!("mdns watch removed: {:?}", fullname);
                        if let Some(peer_id) = self.fullname_map.remove(&fullname) {
                            match self.discovered.remove(&peer_id) {
                                Some(service_info) => {
                                    return Poll::Ready(Ok(DiscoveryEvent::Expired(service_info)));
                                }
                                None => {}
                            }
                        }
                        continue;
                    }
                    e => {
                        tracing::trace!("mdns watch event: {:?}", e);
                        continue;
                    }
                },
                Poll::Ready(None) => {
                    return Poll::Ready(Err(RegistryError::Closed));
                }
                Poll::Pending => {}
            }

            return Poll::Pending;
        }
    }
}

impl TryFrom<mdns_sd::ServiceInfo> for ServiceInfo {
    type Error = RegistryError;

    fn try_from(info: mdns_sd::ServiceInfo) -> Result<ServiceInfo, Self::Error> {
        let peer = info
            .get_property_val_str(PROPERTY_PEER_ID)
            .ok_or(RegistryError::PeerIdNotFound)?;
        let name = info.get_hostname().replace(".local.", "").to_string();
        let peer = if info.get_fullname().ends_with(SERVICE_NAME_FQDN) {
            // 替换掉 FQDN 后缀
            peer.trim_end_matches(SERVICE_NAME_FQDN)
        } else {
            return Err(RegistryError::PeerIdNotFound);
        };

        //读取Address
        let addresses = info
            .get_properties()
            .iter()
            .filter_map(|v| {
                if v.key().starts_with(PROPERTY_ADDR_PREFIX) {
                    Some(v.val_str().to_string())
                } else {
                    None
                }
            })
            .map(|addr| addr.parse::<Multiaddr>())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ServiceInfo {
            name,
            peer_id: PeerId::try_from_base58(peer)?,
            addresses,
            metadata: HashMap::new(),
            ttl: Duration::from_secs(info.get_host_ttl() as u64),
        })
    }
}

impl TryFrom<ServiceInfo> for mdns_sd::ServiceInfo {
    type Error = RegistryError;

    fn try_from(service_info: ServiceInfo) -> Result<Self, Self::Error> {
        let peer_id = service_info.peer_id.into_base58();
        let mut properties: HashMap<String, String> = HashMap::new();
        properties.insert(PROPERTY_PEER_ID.to_string(), peer_id.clone());
        for (key, value) in service_info.metadata {
            properties.insert(key, value);
        }
        for (id, addr) in service_info.addresses.iter().enumerate() {
            properties.insert(format!("{}{}", PROPERTY_ADDR_PREFIX, id), addr.to_string());
        }
        let mut info = mdns_sd::ServiceInfo::new(
            SERVICE_NAME_FQDN,
            peer_id.as_str(),
            format!("{}.local.", service_info.name).as_str(),
            (),
            0,
            properties,
        )?
        .enable_addr_auto();

        info.set_requires_probe(false);

        Ok(info)
    }
}

impl From<mdns_sd::Error> for RegistryError {
    fn from(error: mdns_sd::Error) -> Self {
        RegistryError::Other(Box::new(error))
    }
}
