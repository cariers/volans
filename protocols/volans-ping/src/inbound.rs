use std::{
    collections::VecDeque,
    convert::Infallible,
    io,
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use futures::{FutureExt, future::BoxFuture};
use futures_timer::Delay;
use volans_core::{PeerId, Url, upgrade::ReadyUpgrade};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId,
    InboundStreamHandler, InboundUpgradeSend, NetworkBehavior, NetworkIncomingBehavior,
    StreamProtocol, Substream, SubstreamProtocol, THandlerAction, THandlerEvent,
};

use crate::{Config, Event, Failure, protocol};

type PongFuture = BoxFuture<'static, Result<Substream, io::Error>>;

pub struct Handler {
    interval: Delay,
    config: Config,
    last_ping: Instant,
    failed: bool,
    inbound: Option<PongFuture>,
    pending_errors: VecDeque<Failure>,
}

impl Handler {
    pub fn new(config: Config) -> Self {
        Self {
            interval: Delay::new(config.interval * config.failures),
            config,
            last_ping: Instant::now(),
            failed: false,
            inbound: None,
            pending_errors: VecDeque::new(),
        }
    }
}

impl ConnectionHandler for Handler {
    type Action = Infallible;

    type Event = Result<Duration, Failure>;

    fn handle_action(&mut self, _action: Self::Action) {
        unreachable!("Ping handler does not support actions");
    }

    fn poll_close(&mut self, _: &mut Context<'_>) -> Poll<Option<Self::Event>> {
        if let Some(error) = self.pending_errors.pop_back() {
            return Poll::Ready(Some(Err(error)));
        }
        Poll::Ready(None)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<ConnectionHandlerEvent<Self::Event>> {
        loop {
            if let Some(error) = self.pending_errors.pop_back() {
                return Poll::Ready(ConnectionHandlerEvent::Notify(Err(error)));
            }

            if self.failed {
                return Poll::Ready(ConnectionHandlerEvent::CloseConnection);
            }

            if let Some(fut) = self.inbound.as_mut() {
                match fut.poll_unpin(cx) {
                    Poll::Pending => {}
                    Poll::Ready(Ok(substream)) => {
                        //重新开始新的延迟
                        self.inbound = Some(protocol::recv_ping(substream).boxed());
                        // 重置为新的周期间隔
                        self.interval
                            .reset(self.config.interval * self.config.failures);

                        let elapsed = self.last_ping.elapsed();
                        self.last_ping = Instant::now();

                        return Poll::Ready(ConnectionHandlerEvent::Notify(Ok(elapsed)));
                    }
                    Poll::Ready(Err(err)) => {
                        self.inbound = None;
                        self.failed = true;
                        self.pending_errors.push_back(Failure::other(err));
                        continue;
                    }
                }
            }

            match self.interval.poll_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(()) => {
                    // 重置为新的周期间隔
                    tracing::debug!("Ping timeout, sending ping");
                    self.interval
                        .reset(self.config.interval * self.config.failures);
                    self.inbound = None;
                    self.failed = true;
                    self.pending_errors.push_back(Failure::Timeout);
                    continue;
                }
            }
            return Poll::Pending;
        }
    }
}

impl InboundStreamHandler for Handler {
    type InboundUpgrade = ReadyUpgrade<StreamProtocol>;

    type InboundUserData = ();

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundUpgrade, Self::InboundUserData> {
        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ())
    }

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::InboundUserData,
        protocol: <Self::InboundUpgrade as InboundUpgradeSend>::Output,
    ) {
        self.inbound = Some(protocol::recv_ping(protocol).boxed());
        self.last_ping = Instant::now();
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::InboundUserData,
        error: <Self::InboundUpgrade as InboundUpgradeSend>::Error,
    ) {
        tracing::debug!("Ping protocol upgrade error: {}", error);
        self.inbound = None;
        self.interval.reset(Duration::new(0, 0));
    }
}

pub struct Behavior {
    config: Config,
    events: VecDeque<Event>,
    none_event_waker: Option<Waker>,
}

impl Behavior {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            events: VecDeque::new(),
            none_event_waker: None,
        }
    }
}

impl Default for Behavior {
    fn default() -> Self {
        Self::new(Config::default())
    }
}

impl NetworkBehavior for Behavior {
    type ConnectionHandler = Handler;
    type Event = Event;

    fn on_connection_handler_event(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        event: THandlerEvent<Self>,
    ) {
        self.events.push_front(Event {
            peer_id,
            connection: id,
            result: event,
        });
        if let Some(waker) = self.none_event_waker.take() {
            waker.wake();
        }
    }

    fn poll(
        &mut self,
        _cx: &mut Context<'_>,
    ) -> Poll<BehaviorEvent<Self::Event, THandlerAction<Self>>> {
        if let Some(event) = self.events.pop_back() {
            return Poll::Ready(BehaviorEvent::Behavior(event));
        }
        self.none_event_waker = Some(_cx.waker().clone());
        Poll::Pending
    }
}

impl NetworkIncomingBehavior for Behavior {
    /// 处理已建立的连接
    fn handle_established_connection(
        &mut self,
        _id: ConnectionId,
        peer_id: PeerId,
        _local_addr: &Url,
        _remote_addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        tracing::trace!("Ping handler established for peer: {}", peer_id);
        Ok(Handler::new(self.config.clone()))
    }
}
