use std::{
    collections::VecDeque,
    fmt, io,
    task::{Context, Poll},
    time::Duration,
};

use futures::{AsyncWriteExt, FutureExt};
use futures_bounded::{Delay, FuturesMap};
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamUpgradeError, SubstreamProtocol,
};

use crate::{Codec, RequestId, Upgrade};

pub struct Handler<TCodec>
where
    TCodec: Codec,
{
    codec: TCodec,
    pending_outbound: VecDeque<OutboundRequest<TCodec>>,
    requested_outbound: VecDeque<OutboundRequest<TCodec>>,
    pending_events: VecDeque<Event<TCodec>>,
    requesting: FuturesMap<RequestId, Result<Event<TCodec>, io::Error>>,
}

impl<TCodec> Handler<TCodec>
where
    TCodec: Codec + Send + 'static,
{
    pub fn new(codec: TCodec, stream_timeout: Duration) -> Self {
        Self {
            codec,
            pending_outbound: VecDeque::new(),
            requested_outbound: VecDeque::new(),
            pending_events: VecDeque::new(),
            requesting: FuturesMap::new(move || Delay::futures_timer(stream_timeout), 10),
        }
    }
}

pub enum Event<TCodec>
where
    TCodec: Codec,
{
    Response {
        request_id: RequestId,
        response: TCodec::Response,
    },
    Unsupported(RequestId),
    Timeout(RequestId),
    StreamError {
        request_id: RequestId,
        error: io::Error,
    },
}

impl<TCodec> fmt::Debug for Event<TCodec>
where
    TCodec: Codec,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Response { request_id, .. } => f
                .debug_struct("Response")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Event::Unsupported(request_id) => f
                .debug_struct("UnsupportedProtocol")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Event::Timeout(request_id) => f
                .debug_struct("Timeout")
                .field("request_id", request_id)
                .finish_non_exhaustive(),
            Event::StreamError { request_id, error } => f
                .debug_struct("StreamError")
                .field("request_id", request_id)
                .field("error", error)
                .finish_non_exhaustive(),
        }
    }
}

pub struct OutboundRequest<TCodec: Codec> {
    pub(crate) request_id: RequestId,
    pub(crate) request: TCodec::Request,
    pub(crate) protocol: TCodec::Protocol,
}

impl<TCodec: Codec> fmt::Debug for OutboundRequest<TCodec> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutboundRequest").finish_non_exhaustive()
    }
}

impl<TCodec> ConnectionHandler for Handler<TCodec>
where
    TCodec: Codec + Send + 'static,
{
    type Action = OutboundRequest<TCodec>;
    type Event = Event<TCodec>;

    fn handle_action(&mut self, action: Self::Action) {
        self.pending_outbound.push_back(action);
    }

    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        if let Some(event) = self.pending_events.pop_front() {
            return Poll::Ready(Some(event));
        }
        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        match self.requesting.poll_unpin(cx) {
            Poll::Ready((_, Ok(Ok(event)))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(event));
            }
            Poll::Ready((request_id, Ok(Err(error)))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Event::StreamError {
                    request_id,
                    error,
                }));
            }
            Poll::Ready((request_id, Err(_))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Event::Timeout(request_id)));
            }
            Poll::Pending => {}
        }
        if let Some(event) = self.pending_events.pop_front() {
            return Poll::Ready(ConnectionHandlerEvent::Notify(event));
        }
        Poll::Pending
    }
}

impl<TCodec> OutboundStreamHandler for Handler<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    type OutboundUpgrade = Upgrade<TCodec::Protocol>;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        (mut stream, protocol): <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        let message = self
            .requested_outbound
            .pop_front()
            .expect("negotiated a stream without a pending message");

        let mut codec = self.codec.clone();
        let request_id = message.request_id;

        let fut = async move {
            let write = codec.write_request(&protocol, &mut stream, message.request);
            write.await?;
            stream.close().await?;
            let read = codec.read_response(&protocol, &mut stream);
            let response = read.await?;

            Ok(Event::Response {
                request_id,
                response,
            })
        };

        if self.requesting.try_push(request_id, fut.boxed()).is_err() {
            self.pending_events.push_back(Event::StreamError {
                request_id,
                error: io::Error::other("max sub-streams reached"),
            });
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        let outbound = self
            .requested_outbound
            .pop_front()
            .expect("negotiated a stream without a pending message");

        match error {
            StreamUpgradeError::Timeout => {
                self.pending_events
                    .push_back(Event::Timeout(outbound.request_id));
            }
            StreamUpgradeError::NegotiationFailed => {
                self.pending_events
                    .push_back(Event::Unsupported(outbound.request_id));
            }
            StreamUpgradeError::Apply(_) => {}
            StreamUpgradeError::Io(error) => {
                self.pending_events.push_back(Event::StreamError {
                    request_id: outbound.request_id,
                    error: error,
                });
            }
        }
    }

    fn poll_outbound_request(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        if let Some(request) = self.pending_outbound.pop_front() {
            let protocol = request.protocol.clone();
            self.requested_outbound.push_back(request);
            return Poll::Ready(SubstreamProtocol::new(Upgrade::new_single(protocol), ()));
        }
        Poll::Pending
    }
}
