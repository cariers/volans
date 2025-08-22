use std::{
    collections::{HashMap, HashSet, VecDeque},
    convert::Infallible,
    io,
    task::{Context, Poll},
    time::Duration,
};

use either::Either;
use futures::{StreamExt, channel::mpsc};
use volans_core::{Multiaddr, PeerId};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, PeerCondition, THandlerAction, THandlerEvent,
    behavior::NotifyHandler,
    error::{ConnectionError, DialError},
    handler::DummyHandler,
};

use crate::{MultiaddrExt, transport::TransportRequest};

use super::handler;
pub struct Behavior {
    transport_receiver: mpsc::Receiver<TransportRequest>,
    direct_connections: HashMap<PeerId, HashSet<ConnectionId>>,
    pending_channels: HashMap<PeerId, VecDeque<handler::NewOutboundBridgeRequest>>,
    dial_peers: VecDeque<(PeerId, Option<Multiaddr>)>,
    pending_events: VecDeque<BehaviorEvent<Infallible, THandlerAction<Self>>>,
    timeout: Duration,
}

impl Behavior {
    pub fn new(transport_receiver: mpsc::Receiver<TransportRequest>) -> Self {
        Self {
            transport_receiver,
            direct_connections: HashMap::new(),
            pending_channels: HashMap::new(),
            dial_peers: VecDeque::new(),
            pending_events: VecDeque::new(),
            timeout: Duration::from_secs(15), // Default timeout for outbound requests
        }
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = Either<DummyHandler, handler::Handler>;
    type Event = Infallible;

    fn on_connection_handler_event(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _event: THandlerEvent<Self>,
    ) {
        unreachable!("This behavior does not handle connection events directly.");
    }

    fn poll(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        loop {
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(event);
            }
            match self.transport_receiver.poll_next_unpin(cx) {
                Poll::Ready(Some(TransportRequest::DialRequest {
                    relay_addr,
                    relay_peer_id,
                    dst_peer_id,
                    send_back,
                })) => {
                    let connection_id = self
                        .direct_connections
                        .get(&relay_peer_id)
                        .and_then(|set| set.iter().next())
                        .cloned();

                    if let Some(connection_id) = connection_id {
                        tracing::debug!(
                            "Sending request to relay peer: {:?}, connection ID: {:?}",
                            relay_peer_id,
                            connection_id
                        );
                        return Poll::Ready(BehaviorEvent::HandlerAction {
                            peer_id: relay_peer_id,
                            handler: NotifyHandler::One(connection_id),
                            action: Either::Right(handler::NewOutboundBridgeRequest {
                                dst_addresses: vec![],
                                dst_peer_id,
                                send_back,
                            }),
                        });
                    } else {
                        tracing::debug!(
                            "No direct connection to relay peer: {:?}, attempting to dial",
                            relay_peer_id
                        );
                        self.pending_channels
                            .entry(relay_peer_id)
                            .or_default()
                            .push_back(handler::NewOutboundBridgeRequest {
                                dst_addresses: vec![],
                                dst_peer_id,
                                send_back,
                            });
                        self.dial_peers.push_back((relay_peer_id, Some(relay_addr)));
                        continue;
                    }
                }
                Poll::Ready(Some(TransportRequest::ListenRequest { .. })) => {
                    tracing::error!("Unexpected ListenRequest in Behavior");
                }
                Poll::Pending | Poll::Ready(None) => {}
            }
            return Poll::Pending;
        }
    }
}

impl NetworkOutgoingBehavior for Behavior {
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        if !addr.is_circuit() {
            // 如果是待处理的请求，返回对应的处理器
            Ok(Either::Right(handler::Handler::new(self.timeout)))
        } else {
            // 否则返回一个空的处理器
            Ok(Either::Left(DummyHandler))
        }
    }

    fn on_connection_established(&mut self, id: ConnectionId, peer_id: PeerId, addr: &Multiaddr) {
        // 在排队中的连接
        tracing::warn!(
            "Connection established with peer: {:?}, addr: {:?}",
            peer_id,
            addr
        );

        if !addr.is_circuit() {
            self.direct_connections
                .entry(peer_id)
                .or_default()
                .insert(id);

            // 处理拨号成功，移出正在排队的请求
            if let Some(mut requests) = self.pending_channels.remove(&peer_id) {
                while let Some(request) = requests.pop_front() {
                    let connection_id = self
                        .direct_connections
                        .get(&peer_id)
                        .and_then(|set| set.iter().next())
                        .cloned()
                        .expect("Connection ID should exist after connection established");
                    self.pending_events.push_back(BehaviorEvent::HandlerAction {
                        peer_id,
                        handler: NotifyHandler::One(connection_id),
                        action: Either::Right(request),
                    });
                }
            }
        }
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
        if !addr.is_circuit() {
            if let Some(connections) = self.direct_connections.get_mut(&peer_id) {
                connections.remove(&id);
                if connections.is_empty() {
                    self.direct_connections.remove(&peer_id);
                }
            }
        }
    }

    fn on_dial_failure(
        &mut self,
        _id: ConnectionId,
        peer_id: Option<PeerId>,
        addr: Option<&Multiaddr>,
        error: &DialError,
    ) {
        tracing::warn!(
            "Dial failure occurred peer id: {:?}, addr: {:?}, error: {:?}",
            peer_id,
            addr,
            error,
        );
        if let Some(peer_id) = peer_id {
            if let Some(requests) = self.pending_channels.get_mut(&peer_id) {
                if let Some(request) = requests.pop_front() {
                    tracing::error!("Dial failed for request: {:?}", request);
                    let _ = request.send_back.send(Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Dial failed",
                    )
                    .into()));
                }
            }
        }
    }

    fn poll_dial(&mut self, _cx: &mut Context<'_>) -> Poll<DialOpts> {
        if let Some((peer, relay_addr)) = self.dial_peers.pop_front() {
            tracing::debug!("Dialing Bridge peer: {:?}", peer);
            return Poll::Ready(
                DialOpts::new(relay_addr, Some(peer))
                    .with_condition(PeerCondition::DisconnectedAndNotDialing),
            );
        }
        Poll::Pending
    }
}
