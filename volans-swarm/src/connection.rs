mod inbound;
mod outbound;

pub mod pool;

pub use inbound::InboundConnection;
pub use outbound::OutboundConnection;
pub use pool::{EstablishedConnection, Pool, PoolConfig, PoolEvent};

use std::{
    fmt, mem,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
    time::{Duration, Instant},
};

use futures::{FutureExt, Stream, future::BoxFuture};
use futures_timer::Delay;
use volans_core::muxing::{Closing, StreamMuxerBox, SubstreamBox};
use volans_stream_select::{NegotiationError, ProtocolError};

use crate::{
    ConnectionHandler, InboundUpgradeSend, OutboundUpgradeSend, StreamUpgradeError, Substream,
    SubstreamProtocol, error::ConnectionError, substream::ActiveStreamCounter,
};

static NEXT_CONNECTION_ID: AtomicUsize = AtomicUsize::new(1);

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConnectionId(usize);

impl ConnectionId {
    pub(crate) fn next() -> Self {
        Self(NEXT_CONNECTION_ID.fetch_add(1, Ordering::SeqCst))
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub(crate) trait ConnectionController<THandler: ConnectionHandler> {
    fn close(
        self,
    ) -> (
        Pin<Box<dyn Stream<Item = <THandler as ConnectionHandler>::Event> + Send>>,
        Closing<StreamMuxerBox>,
    );

    fn handle_action(&mut self, action: THandler::Action);

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<THandler::Event, ConnectionError>>;
}

struct StreamUpgrade<TData, TOk, TErr> {
    user_data: Option<TData>,
    timeout: Delay,
    upgrade: BoxFuture<'static, Result<TOk, StreamUpgradeError<TErr>>>,
}

impl<TData, TOk, TErr> StreamUpgrade<TData, TOk, TErr> {
    fn new_outbound<TUpgr>(
        substream: SubstreamBox,
        upgrade: TUpgr,
        user_data: TData,
        timeout: Delay,
        counter: ActiveStreamCounter,
    ) -> Self
    where
        TUpgr: OutboundUpgradeSend<Output = TOk, Error = TErr>,
    {
        let protocols = upgrade.protocol_info();
        Self {
            user_data: Some(user_data),
            timeout,
            upgrade: Box::pin(async move {
                let (info, stream) =
                    volans_stream_select::DialerSelectFuture::new(substream, protocols)
                        .await
                        .map_err(to_stream_upgrade_error)?;
                let output = upgrade
                    .upgrade_outbound(Substream::new(stream, counter), info)
                    .await
                    .map_err(StreamUpgradeError::Apply)?;

                Ok(output)
            }),
        }
    }

    fn new_inbound<TUpgr>(
        substream: SubstreamBox,
        protocol: SubstreamProtocol<TUpgr, TData>,
        counter: ActiveStreamCounter,
    ) -> Self
    where
        TUpgr: InboundUpgradeSend<Output = TOk, Error = TErr>,
    {
        let (upgrade, user_data, timeout) = protocol.into_inner();
        let protocols = upgrade.protocol_info();

        Self {
            user_data: Some(user_data),
            timeout: Delay::new(timeout),
            upgrade: Box::pin(async move {
                let (info, stream) =
                    volans_stream_select::ListenerSelectFuture::new(substream, protocols)
                        .await
                        .map_err(to_stream_upgrade_error)?;
                let output = upgrade
                    .upgrade_inbound(Substream::new(stream, counter), info)
                    .await
                    .map_err(StreamUpgradeError::Apply)?;

                Ok(output)
            }),
        }
    }
}

fn to_stream_upgrade_error<T>(e: NegotiationError) -> StreamUpgradeError<T> {
    match e {
        NegotiationError::Failed => StreamUpgradeError::NegotiationFailed,
        NegotiationError::ProtocolError(ProtocolError::IoError(e)) => StreamUpgradeError::Io(e),
        NegotiationError::ProtocolError(other) => {
            StreamUpgradeError::Io(std::io::Error::other(other))
        }
    }
}

impl<TData, TOk, TErr> Unpin for StreamUpgrade<TData, TOk, TErr> {}

impl<TData, TOk, TErr> Future for StreamUpgrade<TData, TOk, TErr> {
    type Output = (TData, Result<TOk, StreamUpgradeError<TErr>>);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.timeout.poll_unpin(cx) {
            Poll::Ready(()) => {
                return Poll::Ready((
                    self.user_data
                        .take()
                        .expect("Future not to be polled again once ready."),
                    Err(StreamUpgradeError::Timeout),
                ));
            }

            Poll::Pending => {}
        }
        let result = futures::ready!(self.upgrade.poll_unpin(cx));
        let user_data = self
            .user_data
            .take()
            .expect("Future not to be polled again once ready.");

        Poll::Ready((user_data, result))
    }
}

enum SubstreamRequested<TUpgr, TData> {
    Waiting {
        timeout: Delay,
        upgrade: TUpgr,
        user_data: TData,
        extracted_waker: Option<Waker>,
    },
    Done,
}

impl<TUpgr, TData> SubstreamRequested<TUpgr, TData> {
    fn new(upgrade: TUpgr, user_data: TData, timeout: Duration) -> Self {
        Self::Waiting {
            timeout: Delay::new(timeout),
            upgrade,
            user_data,
            extracted_waker: None,
        }
    }
    fn extract(&mut self) -> (TUpgr, TData, Delay) {
        match mem::replace(self, Self::Done) {
            SubstreamRequested::Waiting {
                timeout,
                upgrade,
                extracted_waker: waker,
                user_data,
            } => {
                if let Some(waker) = waker {
                    waker.wake();
                }
                (upgrade, user_data, timeout)
            }
            SubstreamRequested::Done => panic!("cannot extract twice"),
        }
    }
}

impl<TUpgr, TData> Unpin for SubstreamRequested<TUpgr, TData> {}

impl<TUpgr, TData> Future for SubstreamRequested<TUpgr, TData> {
    type Output = Result<(), TData>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match mem::replace(this, SubstreamRequested::Done) {
            Self::Waiting {
                mut timeout,
                user_data,
                upgrade,
                ..
            } => match timeout.poll_unpin(cx) {
                Poll::Ready(()) => Poll::Ready(Err(user_data)),
                Poll::Pending => {
                    *this = SubstreamRequested::Waiting {
                        timeout,
                        upgrade,
                        user_data,
                        extracted_waker: Some(cx.waker().clone()),
                    };
                    Poll::Pending
                }
            },
            Self::Done => Poll::Ready(Ok(())),
        }
    }
}

#[derive(Debug)]
enum Shutdown {
    None,
    /// 尽快关闭
    Asap,
    /// 计划在 `Delay` 结束时关闭
    Later(Delay),
}

fn compute_new_shutdown(
    handler_keep_alive: bool,
    current_shutdown: &Shutdown,
    idle_timeout: Duration,
) -> Option<Shutdown> {
    match (current_shutdown, handler_keep_alive) {
        (_, false) if idle_timeout == Duration::ZERO => Some(Shutdown::Asap),
        (Shutdown::Later(_), false) => None,
        (_, false) => {
            let now = Instant::now();
            let safe_keep_alive = checked_add_fraction(now, idle_timeout);

            Some(Shutdown::Later(Delay::new(safe_keep_alive)))
        }
        (_, true) => Some(Shutdown::None),
    }
}

fn checked_add_fraction(start: Instant, mut duration: Duration) -> Duration {
    while start.checked_add(duration).is_none() {
        tracing::debug!(start=?start, duration=?duration, "start + duration cannot be presented, halving duration");
        duration /= 2;
    }
    duration
}
