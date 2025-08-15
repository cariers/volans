pub mod handler;

pub use handler::Handler;

use std::{
    collections::{HashSet, VecDeque},
    task::{Context, Poll},
};

use smallvec::SmallVec;
use volans_core::{PeerId, Multiaddr};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionId, ListenerEvent, NetworkBehavior,
    NetworkIncomingBehavior, THandlerAction, THandlerEvent,
    error::{ConnectionError, ListenError},
};

use crate::{Codec, Config, InboundFailure, RequestId, Responder};

pub struct Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    protocols: SmallVec<[TCodec::Protocol; 2]>,
    codec: TCodec,
    config: Config,
    pending_event: VecDeque<Event<TCodec::Request, TCodec::Response>>,
    pending_response: HashSet<RequestId>,
}

impl<TCodec> Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    pub fn with_codec<P>(codec: TCodec, protocols: P, config: Config) -> Self
    where
        P: IntoIterator<Item = TCodec::Protocol>,
    {
        let protocols: SmallVec<[TCodec::Protocol; 2]> = protocols.into_iter().collect();

        Self {
            codec,
            config,
            protocols,
            pending_event: VecDeque::new(),
            pending_response: HashSet::new(),
        }
    }

    fn remove_pending_response(&mut self, request_id: RequestId) -> bool {
        self.pending_response.remove(&request_id)
    }
}

impl<TCodec> NetworkBehavior for Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    type Event = Event<TCodec::Request, TCodec::Response>;
    type ConnectionHandler = handler::Handler<TCodec>;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        match event {
            handler::Event::Request {
                request_id,
                request,
                sender,
            } => {
                let responder = Responder { tx: sender };
                self.pending_response.insert(request_id);
                self.pending_event.push_back(Event::Request {
                    peer_id,
                    connection_id: id,
                    request_id,
                    request,
                    responder,
                });
            }
            handler::Event::Discard(request_id) => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event.push_back(Event::Failure {
                    peer_id,
                    connection_id: id,
                    request_id,
                    cause: InboundFailure::Discard,
                });
            }
            handler::Event::Response(request_id) => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event.push_back(Event::ResponseSent {
                    peer_id,
                    connection_id: id,
                    request_id,
                });
            }
            handler::Event::Error { request_id, error } => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event.push_back(Event::Failure {
                    peer_id,
                    connection_id: id,
                    request_id,
                    cause: error.into(),
                });
            }
            handler::Event::Timeout(request_id) => {
                let removed = self.remove_pending_response(request_id);
                debug_assert!(removed, "Response for unknown request: {request_id}");
                self.pending_event.push_back(Event::Failure {
                    peer_id,
                    connection_id: id,
                    request_id,
                    cause: InboundFailure::Timeout,
                });
            }
        }
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        if let Some(event) = self.pending_event.pop_front() {
            return Poll::Ready(BehaviorEvent::Behavior(event));
        }
        Poll::Pending
    }
}

#[derive(Debug)]
pub enum Event<TRequest, TResponse> {
    Request {
        peer_id: PeerId,
        connection_id: ConnectionId,
        request_id: RequestId,
        request: TRequest,
        responder: Responder<TResponse>,
    },
    Failure {
        peer_id: PeerId,
        connection_id: ConnectionId,
        request_id: RequestId,
        cause: InboundFailure,
    },
    ResponseSent {
        peer_id: PeerId,
        connection_id: ConnectionId,
        request_id: RequestId,
    },
}

impl<TCodec> NetworkIncomingBehavior for Behavior<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        let handler = handler::Handler::new(
            self.codec.clone(),
            self.protocols.clone(),
            self.config.request_timeout,
        );
        Ok(handler)
    }

    /// 连接处理器事件处理
    fn on_connection_established(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) {
    }

    fn on_connection_closed(
        &mut self,
        _id: ConnectionId,
        _peer_id: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
        _reason: Option<&ConnectionError>,
    ) {
    }

    /// 监听失败事件处理
    fn on_listen_failure(
        &mut self,
        _id: ConnectionId,
        _peer_id: Option<PeerId>,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
        _error: &ListenError,
    ) {
    }

    /// 监听器事件处理
    fn on_listener_event(&mut self, _event: ListenerEvent<'_>) {}
}
