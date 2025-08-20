use std::collections::HashSet;

use volans_core::Multiaddr;

use crate::{ListenerEvent, behavior::NewListenAddr};

#[derive(Debug, Default, Clone)]
pub struct ListenAddresses {
    addresses: HashSet<Multiaddr>,
}

impl ListenAddresses {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Multiaddr> {
        self.addresses.iter()
    }

    pub fn on_listener_event(&mut self, event: &ListenerEvent) -> bool {
        match event {
            ListenerEvent::NewListenAddr(NewListenAddr { addr, .. }) => {
                self.addresses.insert((*addr).clone())
            }
            ListenerEvent::ExpiredListenAddr(expired) => self.addresses.remove(expired.addr),
            _ => false,
        }
    }
}
