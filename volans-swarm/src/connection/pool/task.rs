use std::{convert::Infallible, pin::Pin};

use futures::{
    SinkExt, StreamExt,
    channel::{mpsc, oneshot},
    future,
};
use volans_core::{PeerId, TransportError, Url, muxing::StreamMuxerBox};

use crate::{
    ConnectionHandler, ConnectionId,
    connection::ConnectionController,
    error::{ConnectionError, PendingConnectionError},
};

#[derive(Debug)]
pub(crate) enum Command<TAction> {
    Action(TAction),
    Close,
}

pub(crate) enum PendingConnectionEvent {
    ConnectionEstablished {
        id: ConnectionId,
        peer_id: PeerId,
        muxer: StreamMuxerBox,
    },
    PendingFailed {
        id: ConnectionId,
        error: PendingConnectionError,
    },
}

impl PendingConnectionEvent {
    pub(crate) fn id(&self) -> &ConnectionId {
        match self {
            PendingConnectionEvent::ConnectionEstablished { id, .. } => id,
            PendingConnectionEvent::PendingFailed { id, .. } => id,
        }
    }
}

#[derive(Debug)]
pub(crate) enum EstablishedConnectionEvent<TEvent> {
    Notify {
        id: ConnectionId,
        peer_id: PeerId,
        event: TEvent,
    },
    Closed {
        id: ConnectionId,
        peer_id: PeerId,
        error: Option<ConnectionError>,
    },
}

pub(crate) async fn new_for_pending_connection<TFut>(
    connection_id: ConnectionId,
    addr: Url,
    future: TFut,
    abort_receiver: oneshot::Receiver<Infallible>,
    mut events: mpsc::Sender<PendingConnectionEvent>,
) where
    TFut: Future<Output = Result<(PeerId, StreamMuxerBox), std::io::Error>> + Send + 'static,
{
    match future::select(abort_receiver, Box::pin(future)).await {
        future::Either::Left((Err(oneshot::Canceled), _)) => {
            let _ = events
                .send(PendingConnectionEvent::PendingFailed {
                    id: connection_id,
                    error: PendingConnectionError::Aborted,
                })
                .await;
        }
        future::Either::Left((Ok(v), _)) => unreachable!("Unexpected abort: {v:?}"),
        future::Either::Right((Ok((peer_id, muxer)), _)) => {
            let _ = events
                .send(PendingConnectionEvent::ConnectionEstablished {
                    id: connection_id,
                    peer_id,
                    muxer,
                })
                .await;
        }
        future::Either::Right((Err(e), _)) => {
            let _ = events
                .send(PendingConnectionEvent::PendingFailed {
                    id: connection_id,
                    error: PendingConnectionError::Transport {
                        addr,
                        error: TransportError::Other(e),
                    },
                })
                .await;
        }
    }
}

pub(crate) async fn new_for_established_connection<THandler, TConnection>(
    connection_id: ConnectionId,
    peer_id: PeerId,
    mut connection: TConnection,
    mut command_receiver: mpsc::Receiver<Command<THandler::Action>>,
    mut events: mpsc::Sender<EstablishedConnectionEvent<THandler::Event>>,
) where
    THandler: ConnectionHandler,
    TConnection: ConnectionController<THandler> + Unpin,
{
    loop {
        match future::select(
            command_receiver.next(),
            future::poll_fn(|cx| Pin::new(&mut connection).poll(cx)),
        )
        .await
        {
            future::Either::Left((Some(command), _)) => match command {
                Command::Action(action) => connection.handle_action(action),
                Command::Close => {
                    // 底层连接错误
                    command_receiver.close();
                    let (remaining_events, closing_muxer) = connection.close();

                    let _ = events
                        .send_all(&mut remaining_events.map(|event| {
                            Ok(EstablishedConnectionEvent::Notify {
                                id: connection_id,
                                event,
                                peer_id,
                            })
                        }))
                        .await;

                    let error = closing_muxer.await.err().map(ConnectionError::Io);
                    let _ = events
                        .send(EstablishedConnectionEvent::Closed {
                            id: connection_id,
                            peer_id,
                            error,
                        })
                        .await;
                    return;
                }
            },
            future::Either::Left((None, _)) => return,
            future::Either::Right((Ok(event), _)) => {
                // 处理连接事件
                let _ = events
                    .send(EstablishedConnectionEvent::Notify {
                        id: connection_id,
                        peer_id,
                        event,
                    })
                    .await;
            }
            future::Either::Right((Err(err), _)) => {
                // 底层连接错误
                command_receiver.close();
                let (remaining_events, _closing_muxer) = connection.close();
                let _ = events
                    .send_all(&mut remaining_events.map(|event| {
                        Ok(EstablishedConnectionEvent::Notify {
                            id: connection_id,
                            event,
                            peer_id,
                        })
                    }))
                    .await;

                let _ = events
                    .send(EstablishedConnectionEvent::Closed {
                        id: connection_id,
                        peer_id,
                        error: Some(err),
                    })
                    .await;
                return;
            }
        }
    }
}
