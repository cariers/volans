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
    ConnectionHandler, ConnectionHandlerEvent, OutboundStreamHandler, OutboundUpgradeSend,
    StreamUpgradeError,
    connection::{
        ConnectionController, Shutdown, StreamUpgrade, SubstreamRequested, compute_new_shutdown,
    },
    error::ConnectionError,
    substream::ActiveStreamCounter,
};

pub struct OutboundConnection<THandler>
where
    THandler: OutboundStreamHandler,
{
    muxer: StreamMuxerBox,
    handler: THandler,
    negotiating_out: FuturesUnordered<
        StreamUpgrade<
            THandler::OutboundUserData,
            <THandler::OutboundUpgrade as OutboundUpgradeSend>::Output,
            <THandler::OutboundUpgrade as OutboundUpgradeSend>::Error,
        >,
    >,
    requested_substreams:
        FuturesUnordered<SubstreamRequested<THandler::OutboundUpgrade, THandler::OutboundUserData>>,

    stream_counter: ActiveStreamCounter,
    closing: bool,
    idle_timeout: Duration,
    shutdown: Shutdown,
}

impl<THandler> Unpin for OutboundConnection<THandler> where THandler: OutboundStreamHandler {}

impl<THandler> OutboundConnection<THandler>
where
    THandler: OutboundStreamHandler,
{
    pub fn new(muxer: StreamMuxerBox, handler: THandler, idle_timeout: Duration) -> Self {
        Self {
            muxer,
            handler,
            negotiating_out: FuturesUnordered::new(),
            requested_substreams: FuturesUnordered::new(),
            stream_counter: ActiveStreamCounter::new(),
            closing: false,
            idle_timeout,
            shutdown: Shutdown::None,
        }
    }

    pub fn is_closing(&self) -> bool {
        self.closing
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
            negotiating_out,
            requested_substreams,
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
            // 检查子流请求是否超时或是完成
            match requested_substreams.poll_next_unpin(cx) {
                // 子流请求完成
                Poll::Ready(Some(Ok(()))) => continue,
                // 子流请求超时
                Poll::Ready(Some(Err(user_data))) => {
                    handler.on_upgrade_error(user_data, StreamUpgradeError::Timeout);
                    continue;
                }
                Poll::Ready(None) | Poll::Pending => {}
            }

            match handler.poll_outbound_request(cx) {
                Poll::Pending => {}
                Poll::Ready(protocol) => {
                    let (upgrade, user_data, timeout) = protocol.into_inner();
                    let substream = SubstreamRequested::new(upgrade, user_data, timeout);
                    requested_substreams.push(substream);
                    continue;
                }
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

            match negotiating_out.poll_next_unpin(cx) {
                Poll::Pending | Poll::Ready(None) => {}
                Poll::Ready(Some((info, Ok(protocol)))) => {
                    handler.on_fully_negotiated(info, protocol);
                    continue;
                }
                Poll::Ready(Some((info, Err(error)))) => {
                    handler.on_upgrade_error(info, error);
                    continue;
                }
            }

            if negotiating_out.is_empty()
                && requested_substreams.is_empty()
                && stream_counter.no_active_streams()
            {
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
            if let Some(requested_substream) = requested_substreams.iter_mut().next() {
                match muxer.poll_outbound_unpin(cx)? {
                    Poll::Pending => {}
                    Poll::Ready(substream) => {
                        let (upgrade, user_data, timeout) = requested_substream.extract();
                        negotiating_out.push(StreamUpgrade::new_outbound(
                            substream,
                            upgrade,
                            user_data,
                            timeout,
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

impl<THandler> ConnectionController<THandler> for OutboundConnection<THandler>
where
    THandler: OutboundStreamHandler,
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
