use std::{
    task::{Context, Poll},
    time::Duration,
};

use futures::FutureExt;
use futures_timer::Delay;
use volans_core::{Multiaddr, PeerId, multiaddr::Protocol};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, ListenAddresses, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, THandlerAction, THandlerEvent, handler::DummyHandler,
};

use crate::{Config, RegisterEvent, Registry, RegistryError, ServiceInfo};

pub struct Behavior<R: Registry> {
    local_peer_id: PeerId,
    registry: R,
    listen_addresses: ListenAddresses,
    pending_register: Option<ServiceInfo>,
    config: Config,
    retry_delay: Option<Delay>,
}

impl<R: Registry> Behavior<R> {
    pub fn new(local_peer_id: PeerId, registry: R, config: Config) -> Self {
        Self {
            local_peer_id,
            registry,
            listen_addresses: ListenAddresses::default(),
            pending_register: None,
            config,
            retry_delay: None,
        }
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
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
        if self.retry_delay.is_none() {
            if let Some(service_info) = self.pending_register.take() {
                match self.registry.register(service_info.clone()) {
                    Ok(()) => {}
                    Err(err) => {
                        self.pending_register = Some(service_info);
                        self.retry_delay = Some(Delay::new(Duration::from_secs(10)));
                        return Poll::Ready(BehaviorEvent::Behavior(Event::RegistryError(err)));
                    }
                }
            }
        }
        if let Some(delay) = &mut self.retry_delay {
            match delay.poll_unpin(cx) {
                Poll::Ready(()) => {
                    self.retry_delay = None;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
        match self.registry.poll_next(cx) {
            Poll::Ready(Ok(RegisterEvent::Registered(service_info))) => {
                Poll::Ready(BehaviorEvent::Behavior(Event::Registered(service_info)))
            }
            Poll::Ready(Ok(RegisterEvent::Deregistered(peer_id))) => {
                Poll::Ready(BehaviorEvent::Behavior(Event::Deregistered(peer_id)))
            }
            Poll::Ready(Err(err)) => {
                Poll::Ready(BehaviorEvent::Behavior(Event::RegistryError(err)))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<R: Registry> NetworkIncomingBehavior for Behavior<R> {
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        Ok(DummyHandler)
    }

    fn on_listener_event(&mut self, event: ListenerEvent<'_>) {
        if self.listen_addresses.on_listener_event(&event) {
            // 监听地址发生变化，可能需要重新注册服务
            let address: Vec<Multiaddr> = self
                .listen_addresses
                .iter()
                .filter(|addr| is_network_address(addr))
                .cloned()
                .collect();

            if address.is_empty() {
                tracing::warn!("No valid network addresses found for registration");
                return;
            }

            let info = ServiceInfo {
                name: self.config.name.clone(),
                peer_id: self.local_peer_id,
                addresses: address,
                metadata: self.config.metadata.clone(),
                ttl: self.config.ttl,
            };
            self.pending_register = Some(info);
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Registered(ServiceInfo),
    Deregistered(PeerId),
    RegistryError(RegistryError),
}

fn is_network_address(addr: &Multiaddr) -> bool {
    let Some(protocol) = addr.iter().next() else {
        return false;
    };
    matches!(
        protocol,
        Protocol::Ip4(_)
            | Protocol::Ip6(_)
            | Protocol::Dns(_)
            | Protocol::Dns4(_)
            | Protocol::Dns6(_)
    )
}
