use std::{
    collections::VecDeque,
    convert::Infallible,
    io, mem,
    task::{Context, Poll, Waker},
    time::Duration,
};

use futures::{
    FutureExt,
    future::{self, BoxFuture},
};
use futures_timer::Delay;
use volans_core::{PeerId, Url, upgrade::ReadyUpgrade};
use volans_swarm::{
    BehaviorEvent, ConnectionDenied, ConnectionHandler, ConnectionHandlerEvent, ConnectionId,
    NetworkBehavior, NetworkOutgoingBehavior, OutboundStreamHandler, OutboundUpgradeSend,
    StreamProtocol, StreamUpgradeError, Substream, SubstreamProtocol, THandlerAction,
    THandlerEvent,
};

use crate::{Config, Event, Failure, protocol};

pub struct Handler {
    interval: Delay,
    config: Config,
    failures: u32,
    outbound: OutboundState,
    pending_errors: VecDeque<Failure>,
    state: State,
}

impl Handler {
    pub fn new(config: Config) -> Self {
        Self {
            interval: Delay::new(config.interval),
            config,
            failures: 0,
            outbound: OutboundState::None,
            pending_errors: VecDeque::new(),
            state: State::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Inactive { reported: bool },
    Active,
}

enum OutboundState {
    None,
    OpenStream,
    Idle(Substream),
    Ping(PingFuture),
}

type PingFuture = BoxFuture<'static, Result<(Substream, Duration), Failure>>;

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
        match self.state {
            State::Inactive { reported: true } => {
                return Poll::Pending;
            }
            State::Inactive { reported: false } => {
                self.state = State::Inactive { reported: true };
                return Poll::Ready(ConnectionHandlerEvent::Notify(Err(Failure::Unsupported)));
            }
            State::Active => {}
        }

        loop {
            if let Some(error) = self.pending_errors.pop_back() {
                self.failures += 1;
                return Poll::Ready(ConnectionHandlerEvent::Notify(Err(error)));
            }

            // 如果失败次数超过配置的最大值，关闭连接
            if self.failures >= self.config.failures {
                return Poll::Ready(ConnectionHandlerEvent::CloseConnection);
            }

            match mem::replace(&mut self.outbound, OutboundState::None) {
                OutboundState::None => {}
                OutboundState::OpenStream => {
                    self.outbound = OutboundState::OpenStream;
                }
                OutboundState::Idle(stream) => match self.interval.poll_unpin(cx) {
                    Poll::Pending => {
                        self.outbound = OutboundState::Idle(stream);
                    }
                    Poll::Ready(()) => {
                        // 间隔到达， State: Idle -> Ping
                        self.outbound =
                            OutboundState::Ping(send_ping(stream, self.config.timeout).boxed());
                        continue;
                    }
                },
                OutboundState::Ping(mut ping) => match ping.poll_unpin(cx) {
                    Poll::Pending => {
                        self.outbound = OutboundState::Ping(ping);
                        return Poll::Pending;
                    }
                    Poll::Ready(Ok((stream, rtt))) => {
                        // Ping 成功，重置失败计数器 State: Ping -> Idle
                        self.failures = 0;
                        self.interval.reset(self.config.interval);
                        self.outbound = OutboundState::Idle(stream);
                        return Poll::Ready(ConnectionHandlerEvent::Notify(Ok(rtt)));
                    }
                    Poll::Ready(Err(e)) => {
                        // Ping 超时或失败 State: Ping -> None
                        self.interval.reset(self.config.interval);
                        self.pending_errors.push_front(e);
                        continue;
                    }
                },
            }
            return Poll::Pending;
        }
    }
}

impl OutboundStreamHandler for Handler {
    type OutboundUpgrade = ReadyUpgrade<StreamProtocol>;
    type OutboundUserData = ();

    fn on_fully_negotiated(
        &mut self,
        _user_data: Self::OutboundUserData,
        stream: <Self::OutboundUpgrade as OutboundUpgradeSend>::Output,
    ) {
        self.outbound = OutboundState::Ping(send_ping(stream, self.config.timeout).boxed());
    }

    fn on_upgrade_error(
        &mut self,
        _user_data: Self::OutboundUserData,
        error: StreamUpgradeError<<Self::OutboundUpgrade as OutboundUpgradeSend>::Error>,
    ) {
        self.outbound = OutboundState::None;
        self.interval.reset(Duration::new(0, 0));
        let error = match error {
            StreamUpgradeError::Timeout => Failure::other(io::Error::new(
                io::ErrorKind::TimedOut,
                "Ping protocol negotiation timed out",
            )),
            StreamUpgradeError::NegotiationFailed => {
                debug_assert_eq!(self.state, State::Active);
                self.state = State::Inactive { reported: false };
                return;
            }
            StreamUpgradeError::Apply(err) => Failure::other(err),
            StreamUpgradeError::Io(err) => Failure::other(err),
        };

        self.pending_errors.push_back(error);
    }

    fn poll_outbound_request(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<SubstreamProtocol<Self::OutboundUpgrade, Self::OutboundUserData>> {
        match self.outbound {
            OutboundState::None => match self.interval.poll_unpin(cx) {
                Poll::Pending => {}
                Poll::Ready(()) => {
                    // 首次间隔到达， State: None -> OpenStream
                    self.outbound = OutboundState::OpenStream;
                    let protocol =
                        SubstreamProtocol::new(ReadyUpgrade::new(protocol::PROTOCOL_NAME), ());
                    return Poll::Ready(protocol);
                }
            },
            _ => {}
        }
        Poll::Pending
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

impl NetworkOutgoingBehavior for Behavior {
    fn handle_established_connection(
        &mut self,
        id: ConnectionId,
        peer_id: PeerId,
        addr: &Url,
    ) -> Result<Self::ConnectionHandler, ConnectionDenied> {
        tracing::trace!(
            "Ping handler established for peer: {}, {}, {}",
            id,
            peer_id,
            addr
        );
        Ok(Handler::new(self.config.clone()))
    }
}

async fn send_ping(stream: Substream, timeout: Duration) -> Result<(Substream, Duration), Failure> {
    let ping = protocol::send_ping(stream);
    futures::pin_mut!(ping);

    match future::select(ping, Delay::new(timeout)).await {
        future::Either::Left((Ok((stream, rtt)), _)) => Ok((stream, rtt)),
        future::Either::Left((Err(e), _)) => Err(Failure::other(e)),
        future::Either::Right(((), _)) => Err(Failure::Timeout),
    }
}
