use std::{
    collections::HashMap,
    task::{Context, Poll},
};

use volans_core::{Multiaddr, PeerId};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, THandlerAction, THandlerEvent, handler::DummyHandler,
};

use crate::{Discovery, DiscoveryEvent, Registry, RegistryError, ServiceInfo};

pub struct Behavior<R: Registry> {
    discovery: R::Discovery,
    discovered: HashMap<PeerId, ServiceInfo>,
}

impl<R: Registry> Default for Behavior<R> {
    fn default() -> Self {
        Self {
            discovered: HashMap::new(),
            discovery: R::default()
                .discovery()
                .expect("Discovery should be available"),
        }
    }
}

impl<R: Registry> NetworkBehavior for Behavior<R> {
    type ConnectionHandler = DummyHandler;
    type Event = Event;

    fn on_connection_handler_event(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        unreachable!("Unexpected event: {:?}", event);
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        match self.discovery.poll_watch(cx) {
            Poll::Ready(Ok(DiscoveryEvent::Discovered(service_info))) => {
                self.discovered
                    .insert(service_info.peer_id, service_info.clone());
                Poll::Ready(BehaviorEvent::Behavior(Event::Discovered(service_info)))
            }
            Poll::Ready(Ok(DiscoveryEvent::Expired(service_info))) => {
                self.discovered.remove(&service_info.peer_id);
                Poll::Ready(BehaviorEvent::Behavior(Event::Expired(service_info)))
            }
            Poll::Ready(Err(err)) => {
                Poll::Ready(BehaviorEvent::Behavior(Event::RegistryError(err)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R: Registry> NetworkOutgoingBehavior for Behavior<R> {
    fn handle_pending_connection(
        &mut self,
        _id: ConnectionId,
        maybe_peer: Option<PeerId>,
        addr: &Option<Multiaddr>,
    ) -> Result<Option<Multiaddr>, ConnectionDenied> {
        if addr.is_some() {
            return Ok(addr.clone());
        }
        match maybe_peer {
            Some(peer_id) => {
                if let Some(service_info) = self.discovered.get(&peer_id) {
                    let addr = service_info.addresses.iter().next().cloned();
                    tracing::debug!("Using address {:?} for peer ID {}", addr, peer_id);
                    return Ok(addr);
                } else {
                    tracing::warn!("Peer ID {} not found in discovered services", peer_id);
                }
            }
            None => {}
        }
        Ok(None)
    }

    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(DummyHandler)
    }

    fn poll_dial(&mut self, cx: &mut Context<'_>) -> Poll<DialOpts> {
        Poll::Pending
    }
}

#[derive(Debug)]
pub enum Event {
    Discovered(ServiceInfo),
    Expired(ServiceInfo),
    RegistryError(RegistryError),
}
