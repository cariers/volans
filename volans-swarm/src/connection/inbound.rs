use std::{
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use futures::{
    Stream, StreamExt,
    stream::{self, FuturesUnordered},
};
use volans_core::muxing::{Closing, StreamMuxerBox, StreamMuxerExt};

use crate::{
    ConnectionHandler, ConnectionHandlerEvent, InboundStreamHandler, InboundUpgradeSend,
    StreamUpgradeError,
    connection::{ConnectionController, Shutdown, StreamUpgrade, compute_new_shutdown},
    error::ConnectionError,
    substream::ActiveStreamCounter,
};

pub struct InboundConnection<THandler>
where
    THandler: InboundStreamHandler,
{
    muxer: StreamMuxerBox,
    handler: THandler,
    negotiating_in: FuturesUnordered<
        StreamUpgrade<
            THandler::InboundUserData,
            <THandler::InboundUpgrade as InboundUpgradeSend>::Output,
            <THandler::InboundUpgrade as InboundUpgradeSend>::Error,
        >,
    >,
    max_negotiating_inbound_streams: usize,
    stream_counter: ActiveStreamCounter,
    closing: bool,
    idle_timeout: Duration,
    shutdown: Shutdown,
}

impl<THandler> Unpin for InboundConnection<THandler> where THandler: InboundStreamHandler {}

impl<THandler> InboundConnection<THandler>
where
    THandler: InboundStreamHandler,
{
    pub fn new(
        muxer: StreamMuxerBox,
        handler: THandler,
        max_negotiating_inbound_streams: usize,
        idle_timeout: Duration,
    ) -> Self {
        Self {
            muxer,
            handler,
            negotiating_in: FuturesUnordered::new(),
            max_negotiating_inbound_streams,
            stream_counter: ActiveStreamCounter::new(),
            closing: false,
            idle_timeout,
            shutdown: Shutdown::None,
        }
    }

    pub fn close(
        self,
    ) -> (
        Pin<Box<dyn Stream<Item = <THandler as ConnectionHandler>::Event> + Send>>,
        Closing<StreamMuxerBox>,
    ) {
        let Self {
            muxer, mut handler, ..
        } = self;

        let stream = stream::poll_fn(move |cx| handler.poll_close(cx)).boxed();

        (stream, muxer.close())
    }

    pub fn handle_action(&mut self, action: THandler::Action) {
        self.handler.handle_action(action);
    }

    #[tracing::instrument(level = "debug", name = "Connection::poll", skip(self, cx))]
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<THandler::Event, ConnectionError>> {
        let Self {
            muxer,
            handler,
            negotiating_in,
            max_negotiating_inbound_streams,
            stream_counter,
            closing,
            idle_timeout,
            shutdown,
            ..
        } = self;
        loop {
            if *closing {
                // 如果连接已关闭，直接返回关闭事件
                return Poll::Ready(Err(ConnectionError::Closing));
            }

            match handler.poll(cx) {
                Poll::Pending => {}
                // 处理器发生事件
                Poll::Ready(ConnectionHandlerEvent::Notify(event)) => {
                    return Poll::Ready(Ok(event));
                }
                // 关闭连接
                Poll::Ready(ConnectionHandlerEvent::CloseConnection) => {
                    *closing = true;
                    continue;
                }
            }

            match negotiating_in.poll_next_unpin(cx) {
                Poll::Pending | Poll::Ready(None) => {}
                Poll::Ready(Some((info, Ok(protocol)))) => {
                    handler.on_fully_negotiated(info, protocol);
                    continue;
                }
                Poll::Ready(Some((info, Err(StreamUpgradeError::Apply(error))))) => {
                    handler.on_upgrade_error(info, error);
                    continue;
                }
                Poll::Ready(Some((_, Err(StreamUpgradeError::Timeout)))) => {
                    tracing::debug!("inbound stream upgrade timed out");
                    continue;
                }
                Poll::Ready(Some((_, Err(StreamUpgradeError::NegotiationFailed)))) => {
                    tracing::debug!("inbound stream upgrade negotiation failed");
                    continue;
                }
                Poll::Ready(Some((_, Err(StreamUpgradeError::Io(error))))) => {
                    tracing::debug!("inbound stream upgrade IO error: {:?}", error);
                    continue;
                }
            }

            if negotiating_in.is_empty() && stream_counter.no_active_streams() {
                if let Some(new_timeout) =
                    compute_new_shutdown(handler.connection_keep_alive(), shutdown, *idle_timeout)
                {
                    *shutdown = new_timeout;
                }
                match shutdown {
                    Shutdown::None => {}
                    Shutdown::Asap => return Poll::Ready(Err(ConnectionError::KeepAliveTimeout)),
                    Shutdown::Later(delay) => match Future::poll(Pin::new(delay), cx) {
                        Poll::Ready(_) => {
                            return Poll::Ready(Err(ConnectionError::KeepAliveTimeout));
                        }
                        Poll::Pending => {}
                    },
                }
            } else {
                *shutdown = Shutdown::None;
            }

            // 处理多路复用器的事件
            match muxer.poll_unpin(cx)? {
                Poll::Pending => {}
                Poll::Ready(()) => {}
            }

            if negotiating_in.len() < *max_negotiating_inbound_streams {
                match muxer.poll_inbound_unpin(cx)? {
                    Poll::Pending => {}
                    Poll::Ready(substream) => {
                        let protocol = handler.listen_protocol();
                        negotiating_in.push(StreamUpgrade::new_inbound(
                            substream,
                            protocol,
                            stream_counter.clone(),
                        ));
                        continue;
                    }
                }
            }

            return Poll::Pending;
        }
    }
}

impl<THandler> ConnectionController<THandler> for InboundConnection<THandler>
where
    THandler: InboundStreamHandler,
{
    fn close(
        self,
    ) -> (
        Pin<Box<dyn Stream<Item = <THandler as ConnectionHandler>::Event> + Send>>,
        Closing<StreamMuxerBox>,
    ) {
        self.close()
    }

    fn handle_action(&mut self, action: THandler::Action) {
        self.handle_action(action)
    }

    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<THandler::Event, ConnectionError>> {
        self.poll(cx)
    }
}
