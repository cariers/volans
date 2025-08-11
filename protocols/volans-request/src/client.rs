pub mod handler;

pub use handler::Handler;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    task::{Context, Poll},
};

use smallvec::SmallVec;
use volans_core::{PeerId, Url};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, DialOpts, NetworkBehavior,
    NetworkOutgoingBehavior, THandlerAction, THandlerEvent,
    behavior::NotifyHandler,
    error::{ConnectionError, DialError},
};

use crate::{Codec, Config, OutboundFailure, RequestId, client::handler::OutboundRequest};

pub struct Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    clients: HashMap<PeerId, SmallVec<[ConnectionId; 2]>>,
    codec: TCodec,
    config: Config,
    pending_event: VecDeque<BehaviorEvent<Event<TCodec::Response>, THandlerAction<Self>>>,
    pending_response: HashSet<RequestId>,
    pending_requests: HashMap<PeerId, SmallVec<[OutboundRequest<TCodec>; 10]>>,
    pending_dial: HashSet<PeerId>,
}

impl<TCodec> Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    pub fn with_codec(codec: TCodec, config: Config) -> Self {
        Self {
            clients: HashMap::new(),
            codec,
            config,
            pending_event: VecDeque::new(),
            pending_response: HashSet::new(),
            pending_requests: HashMap::new(),
            pending_dial: HashSet::new(),
        }
    }

    pub fn send_request(
        &mut self,
        peer_id: PeerId,
        protocol: TCodec::Protocol,
        request: TCodec::Request,
    ) -> RequestId {
        let request_id = RequestId::next();
        let request = OutboundRequest {
            request_id,
            request,
            protocol,
        };
        if let Some(request) = self.try_send_request(&peer_id, request) {
            self.pending_dial.insert(peer_id);
            self.pending_requests
                .entry(peer_id)
                .or_default()
                .push(request);
        }
        request_id
    }

    // 移除Pending Response
    fn remove_pending_response(&mut self, request_id: RequestId) -> bool {
        self.pending_response.remove(&request_id)
    }

    fn try_send_request(
        &mut self,
        peer_id: &PeerId,
        request: OutboundRequest<TCodec>,
    ) -> Option<OutboundRequest<TCodec>> {
        if let Some(connections) = self.clients.get_mut(peer_id) {
            if connections.is_empty() {
                return Some(request);
            }
            let index = request.request_id.0 & connections.len();
            let connection_id = &mut connections[index];
            self.pending_response.insert(request.request_id);
            self.pending_event.push_back(BehaviorEvent::HandlerAction {
                peer_id: *peer_id,
                handler: NotifyHandler::One(*connection_id),
                action: request,
            });
            None
        } else {
            Some(request)
        }
    }
}

#[derive(Debug)]
pub enum Event<TResponse> {
    Response {
        peer_id: PeerId,
        connection_id: ConnectionId,
        request_id: RequestId,
        response: TResponse,
    },
    Failure {
        peer_id: PeerId,
        connection_id: ConnectionId,
        request_id: RequestId,
        cause: OutboundFailure,
    },
}

impl<TCodec> NetworkBehavior for Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    type ConnectionHandler = Handler<TCodec>;
    type Event = Event<TCodec::Response>;
    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        match event {
            handler::Event::Response {
                request_id,
                response,
            } => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event
                    .push_back(BehaviorEvent::Behavior(Event::Response {
                        peer_id,
                        connection_id: id,
                        request_id,
                        response,
                    }));
            }
            handler::Event::Unsupported(request_id) => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event
                    .push_back(BehaviorEvent::Behavior(Event::Failure {
                        peer_id,
                        connection_id: id,
                        request_id,
                        cause: OutboundFailure::UnsupportedProtocols,
                    }));
            }
            handler::Event::StreamError { request_id, error } => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event
                    .push_back(BehaviorEvent::Behavior(Event::Failure {
                        peer_id,
                        connection_id: id,
                        request_id,
                        cause: error.into(),
                    }));
            }
            handler::Event::Timeout(request_id) => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event
                    .push_back(BehaviorEvent::Behavior(Event::Failure {
                        peer_id,
                        connection_id: id,
                        request_id,
                        cause: OutboundFailure::Timeout,
                    }));
            }
        }
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        if let Some(event) = self.pending_event.pop_front() {
            return Poll::Ready(event);
        }
        Poll::Pending
    }
}

impl<TCodec> NetworkOutgoingBehavior for Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        let handler = handler::Handler::new(self.codec.clone(), self.config.request_timeout);
        Ok(handler)
    }

    fn on_connection_established(&mut self, id: ConnectionId, peer_id: PeerId, _addr: &Url) {
        self.clients.entry(peer_id).or_default().push(id);
    }

    fn on_connection_closed(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        _addr: &Url,
        _reason: Option<&ConnectionError>,
    ) {
        self.clients
            .entry(peer_id)
            .or_default()
            .retain(|x| *x != id);
        if self
            .clients
            .get(&peer_id)
            .map(|v| v.is_empty())
            .unwrap_or(false)
        {
            self.clients.remove(&peer_id);
        }
    }

    fn on_dial_failure(
        &mut self,
        id: ConnectionId,
        peer_id: Option<PeerId>,
        _addr: Option<&Url>,
        _error: &DialError,
    ) {
        if let Some(peer) = peer_id {
            if let Some(pending) = self.pending_requests.remove(&peer) {
                for request in pending {
                    let event = Event::Failure {
                        peer_id: peer,
                        connection_id: id,
                        request_id: request.request_id,
                        cause: OutboundFailure::DialFailure,
                    };
                    self.pending_event.push_back(BehaviorEvent::Behavior(event));
                }
            }
        }
    }

    fn poll_dial(&mut self, _cx: &mut Context<'_>) -> Poll<DialOpts> {
        if let Some(peer_id) = self.pending_dial.iter().next().cloned() {
            self.pending_dial.remove(&peer_id);
            Poll::Ready(DialOpts::new(None, Some(peer_id)))
        } else {
            Poll::Pending
        }
    }
}
