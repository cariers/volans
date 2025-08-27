mod dial_opts;
mod executor;
mod substream;

pub mod behavior;
pub mod client;
pub mod connection;
pub mod derive_prelude;
pub mod error;
pub mod handler;
pub mod listener;
pub mod server;
pub mod upgrade;

pub use behavior::{
    BehaviorEvent, ListenAddresses, ListenerEvent, NetworkBehavior, NetworkIncomingBehavior,
    NetworkOutgoingBehavior,
};
pub use connection::ConnectionId;
pub use dial_opts::{DialOpts, PeerCondition};
pub use error::ConnectionDenied;
pub use executor::{ExecSwitch, Executor};
pub use handler::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, OutboundStreamHandler,
    StreamUpgradeError, SubstreamProtocol,
};
pub use listener::{ListenOpts, ListenerId};
pub use substream::{InvalidProtocol, StreamProtocol, Substream};
pub use upgrade::{InboundUpgradeSend, OutboundUpgradeSend, UpgradeInfoSend};
pub use volans_swarm_derive::{NetworkIncomingBehavior, NetworkOutgoingBehavior};

pub type THandler<B> = <B as NetworkBehavior>::ConnectionHandler;
pub type THandlerAction<B> = <THandler<B> as ConnectionHandler>::Action;
pub type THandlerEvent<B> = <THandler<B> as ConnectionHandler>::Event;

use std::task::{Context, Poll};

use smallvec::SmallVec;

use crate::connection::{EstablishedConnection, Pool};

enum PendingNotifyHandler {
    One(ConnectionId),
    Any(SmallVec<[ConnectionId; 10]>),
}

// 通知单个连接
fn notify_one<THandlerAction>(
    connection: &mut EstablishedConnection<THandlerAction>,
    action: THandlerAction,
    cx: &mut Context<'_>,
) -> Option<THandlerAction> {
    match connection.poll_ready(cx) {
        Poll::Pending => Some(action),
        Poll::Ready(Err(())) => None,
        Poll::Ready(Ok(())) => {
            let _ = connection.start_send(action);
            None
        }
    }
}

// 通知任意一个连接
fn notify_any<TBehavior>(
    ids: SmallVec<[ConnectionId; 10]>,
    pool: &mut Pool<TBehavior::ConnectionHandler>,
    action: THandlerAction<TBehavior>,
    cx: &mut Context<'_>,
) -> Option<(SmallVec<[ConnectionId; 10]>, THandlerAction<TBehavior>)>
where
    TBehavior: NetworkBehavior,
{
    let mut pending = SmallVec::new();
    let mut action = Some(action);
    for id in ids.into_iter() {
        if let Some(connection) = pool.get_established(id) {
            match connection.poll_ready(cx) {
                Poll::Pending => {
                    pending.push(id);
                }
                Poll::Ready(Err(())) => {}
                Poll::Ready(Ok(())) => {
                    let action = Option::take(&mut action).expect("Event should be available");
                    connection.start_send(action).expect("Failed to send event");
                    break;
                }
            }
        }
    }
    action.and_then(|action| {
        if !pending.is_empty() {
            Some((pending, action))
        } else {
            None
        }
    })
}
