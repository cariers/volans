use std::{
    convert::Infallible,
    fmt, io,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    AsyncWriteExt, FutureExt, SinkExt, StreamExt,
    channel::{mpsc, oneshot},
};
use futures_bounded::{Delay, FuturesMap};
use smallvec::SmallVec;
use volans_swarm::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    SubstreamProtocol,
};

use crate::{Codec, RequestId, Upgrade};

pub struct Handler<TCodec>
where
    TCodec: Codec,
{
    codec: TCodec,
    protocols: SmallVec<[TCodec::Protocol; 2]>,
    receiver: mpsc::Receiver<(
        RequestId,
        TCodec::Request,
        oneshot::Sender<TCodec::Response>,
    )>,
    sender: mpsc::Sender<(
        RequestId,
        TCodec::Request,
        oneshot::Sender<TCodec::Response>,
    )>,
    requesting: FuturesMap<RequestId, Result<Event<TCodec>, io::Error>>,
}

impl<TCodec> Handler<TCodec>
where
    TCodec: Codec + Send + 'static,
{
    pub fn new(
        codec: TCodec,
        protocols: SmallVec<[TCodec::Protocol; 2]>,
        stream_timeout: Duration,
    ) -> Self {
        let (sender, receiver) = mpsc::channel(0);
        Self {
            codec,
            protocols,
            receiver,
            sender,
            requesting: FuturesMap::new(move || Delay::futures_timer(stream_timeout), 10),
        }
    }
}

pub enum Event<TCodec>
where
    TCodec: Codec,
{
    Request {
        request_id: RequestId,
        request: TCodec::Request,
        sender: oneshot::Sender<TCodec::Response>,
    },
    Error {
        request_id: RequestId,
        error: io::Error,
    },
    Response(RequestId),
    Discard(RequestId),
    Timeout(RequestId),
}

impl<TCodec> fmt::Debug for Event<TCodec>
where
    TCodec: Codec,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Request { request_id, .. } => f
                .debug_struct("InboundEvent::Request")
                .field("request_id", request_id)
                .finish(),
            Event::Error { request_id, error } => f
                .debug_struct("InboundEvent::Error")
                .field("request_id", request_id)
                .field("error", error)
                .finish(),
            Event::Response(request_id) => f
                .debug_struct("InboundEvent::Response")
                .field("request_id", request_id)
                .finish(),
            Event::Discard(request_id) => f
                .debug_struct("InboundEvent::Discard")
                .field("request_id", request_id)
                .finish(),
            Event::Timeout(request_id) => f
                .debug_struct("InboundEvent::Timeout")
                .field("request_id", request_id)
                .finish(),
        }
    }
}

impl<TCodec> ConnectionHandler for Handler<TCodec>
where
    TCodec: Codec + Send + 'static,
{
    type Action = Infallible;
    type Event = Event<TCodec>;

    fn handle_action(&mut self, _action: Self::Action) {
        unreachable!("Request handler does not support actions");
    }

    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        match self.requesting.poll_unpin(cx) {
            Poll::Ready((_, Ok(Ok(event)))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(event));
            }
            Poll::Ready((request_id, Ok(Err(error)))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Event::Error {
                    request_id,
                    error,
                }));
            }
            Poll::Ready((request_id, Err(_))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Event::Timeout(request_id)));
            }
            Poll::Pending => {}
        }

        match self.receiver.poll_next_unpin(cx) {
            Poll::Ready(Some((request_id, request, sender))) => {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Event::Request {
                    request_id,
                    request,
                    sender,
                }));
            }
            Poll::Ready(None) | Poll::Pending => {}
        }

        Poll::Pending
    }
}

impl<TCodec> InboundStreamHandler for Handler<TCodec>
where
    TCodec: Codec + Clone + Send + 'static,
{
    type InboundUpgrade = Upgrade<TCodec::Protocol>;
    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(
            Upgrade {
                protocols: self.protocols.clone(),
            },
            (),
        )
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        (mut stream, protocol): <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        let mut codec = self.codec.clone();
        let request_id = RequestId::next();
        let mut sender = self.sender.clone();
        let fut = async move {
            let (response_sender, response_receiver) = oneshot::channel();
            let request = codec.read_request(&protocol, &mut stream).await?;
            sender
                .send((request_id, request, response_sender))
                .await
                .expect("Request handler sender should not be closed");
            drop(sender);
            if let Ok(response) = response_receiver.await {
                codec
                    .write_response(&protocol, &mut stream, response)
                    .await?;
                stream.close().await?;
                return Ok(Event::Response(request_id));
            } else {
                stream.close().await?;
                return Ok(Event::Discard(request_id));
            }
        };
        match self.requesting.try_push(request_id, fut.boxed()) {
            Ok(()) => {}
            Err(_) => {
                tracing::warn!("Request handler is overloaded, dropping request");
            }
        }
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        _error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        unreachable!("Request handler does not support upgrade errors");
    }
}
