mod stream;

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use async_tungstenite::{
    accept_async_with_config, client_async_with_config,
    tungstenite::{self, http::Uri, protocol::WebSocketConfig},
};
use futures::{FutureExt, TryFutureExt};
use stream::RwStreamSink;
use volans_core::{
    Listener, ListenerEvent, Multiaddr, Transport, TransportError, multiaddr::Protocol,
};
use volans_tcp::TcpStream;

use crate::framed::BytesWebSocketStream;
pub use tungstenite::Error;

mod framed;

#[derive(Debug, Clone)]
pub struct Config {
    pub websocket: WebSocketConfig,
    pub tcp: volans_tcp::Config,
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Config {
    pub fn new() -> Self {
        Self {
            websocket: WebSocketConfig::default(),
            tcp: volans_tcp::Config::default(),
        }
    }

    /// Set [`Self::read_buffer_size`].
    pub fn read_buffer_size(mut self, read_buffer_size: usize) -> Self {
        self.websocket.read_buffer_size = read_buffer_size;
        self
    }

    /// Set [`Self::write_buffer_size`].
    pub fn write_buffer_size(mut self, write_buffer_size: usize) -> Self {
        self.websocket.write_buffer_size = write_buffer_size;
        self
    }

    /// Set [`Self::max_write_buffer_size`].
    pub fn max_write_buffer_size(mut self, max_write_buffer_size: usize) -> Self {
        self.websocket.max_write_buffer_size = max_write_buffer_size;
        self
    }

    /// Set [`Self::max_message_size`].
    pub fn max_message_size(mut self, max_message_size: Option<usize>) -> Self {
        self.websocket.max_message_size = max_message_size;
        self
    }

    /// Set [`Self::max_frame_size`].
    pub fn max_frame_size(mut self, max_frame_size: Option<usize>) -> Self {
        self.websocket.max_frame_size = max_frame_size;
        self
    }

    /// Set [`Self::accept_unmasked_frames`].
    pub fn accept_unmasked_frames(mut self, accept_unmasked_frames: bool) -> Self {
        self.websocket.accept_unmasked_frames = accept_unmasked_frames;
        self
    }
}

type ListenerUpgrade = Pin<
    Box<dyn Future<Output = Result<RwStreamSink<BytesWebSocketStream<TcpStream>>, Error>> + Send>,
>;

impl Transport for Config {
    type Output = RwStreamSink<BytesWebSocketStream<TcpStream>>;
    type Error = tungstenite::Error;
    type Dial = Pin<Box<dyn Future<Output = Result<Self::Output, Self::Error>> + Send>>;
    type Incoming = ListenerUpgrade;
    type Listener = ListenStream;

    fn dial(&self, addr: Multiaddr) -> Result<Self::Dial, TransportError<Self::Error>> {
        let config = self.websocket.clone();
        tracing::debug!("Connecting to WebSocket at {}", addr);
        let (inner_addr, path) =
            parse_ws_addr(&addr).ok_or_else(|| TransportError::NotSupported(addr.clone()))?;

        let path = path.unwrap_or("".to_string());
        let request = path.parse::<Uri>().map_err(tungstenite::Error::from)?;
        tracing::debug!("Connecting to WebSocket at {}", request);

        let dialer = self
            .tcp
            .dial(inner_addr)
            .map_err(|e| e.map(tungstenite::Error::from))?;

        Ok(dialer
            .map_err(tungstenite::Error::from)
            .and_then(move |stream| client_async_with_config(request, stream, Some(config)))
            .map_ok(|(s, response)| {
                tracing::debug!("WebSocket handshake response: {:?}", response);
                BytesWebSocketStream::new(s)
            })
            .map_ok(RwStreamSink::new)
            .boxed())
    }

    fn listen(&self, addr: Multiaddr) -> Result<Self::Listener, TransportError<Self::Error>> {
        let (inner_addr, path) =
            parse_ws_addr(&addr).ok_or_else(|| TransportError::NotSupported(addr.clone()))?;
        let listener = self
            .tcp
            .listen(inner_addr)
            .map_err(|e| e.map(tungstenite::Error::from))?;
        tracing::debug!("Listening for WebSocket connections on {}", addr);
        Ok(ListenStream {
            path: path.map(|r| r.to_string()),
            config: self.websocket.clone(),
            inner: listener,
        })
    }
}

#[pin_project::pin_project]
pub struct ListenStream {
    path: Option<String>,
    config: WebSocketConfig,
    #[pin]
    inner: volans_tcp::ListenStream,
}

impl Listener for ListenStream {
    type Output = RwStreamSink<BytesWebSocketStream<TcpStream>>;
    type Error = tungstenite::Error;
    type Upgrade = ListenerUpgrade;

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_close(cx).map_err(tungstenite::Error::from)
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.project();
        match this.inner.poll_event(cx) {
            Poll::Ready(event) => {
                let config = this.config.clone();
                let event = event
                    .map_upgrade(|u| {
                        u.map_err(Error::from)
                            .and_then(move |stream| accept_async_with_config(stream, Some(config)))
                            .map_ok(BytesWebSocketStream::new)
                            .map_ok(RwStreamSink::new)
                            .boxed()
                    })
                    .map_err(Error::from);
                Poll::Ready(event)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

fn parse_ws_addr(addr: &Multiaddr) -> Option<(Multiaddr, Option<String>)> {
    let mut inner_addr = addr.clone();
    let maybe_path = inner_addr.pop()?;
    match maybe_path {
        Protocol::Path(path) => match inner_addr.pop()? {
            Protocol::Ws => Some((inner_addr, Some(path.to_string()))),
            _ => None,
        },
        Protocol::Ws => Some((inner_addr, None)),
        _ => None,
    }
}
