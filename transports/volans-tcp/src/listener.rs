use volans_core::{Listener, ListenerEvent};
use futures::future::{self, Ready};
use std::{
    io, mem,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::net::TcpListener;
use url::Url;

use crate::TcpStream;

pub struct ListenStream {
    listener_addr: Url,
    pending_event: Option<ListenerEvent<Ready<Result<TcpStream, io::Error>>, io::Error>>,
    state: State,
}

enum State {
    Listening { listener: TcpListener },
    Closed,
}

impl ListenStream {
    pub fn new(listener: TcpListener, listener_addr: Url) -> Self {
        let listened_event = ListenerEvent::Listened(listener_addr.clone());
        ListenStream {
            listener_addr,
            state: State::Listening { listener },
            pending_event: Some(listened_event),
        }
    }
}

impl Listener for ListenStream {
    type Error = io::Error;
    type Output = TcpStream;
    type Upgrade = Ready<Result<TcpStream, io::Error>>;

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.get_mut();
        match mem::replace(&mut this.state, State::Closed) {
            State::Listening { listener } => {
                this.state = State::Closed;
                drop(listener);
                Poll::Ready(Ok(()))
            }
            State::Closed => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, "Listener closed")))
            }
        }
    }

    fn poll_event(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<ListenerEvent<Self::Upgrade, Self::Error>> {
        let this = self.get_mut();
        if let Some(event) = this.pending_event.take() {
            return Poll::Ready(event);
        }
        match &mut this.state {
            State::Listening { listener } => match Pin::new(listener).poll_accept(cx) {
                Poll::Ready(Ok((stream, remote_addr))) => {
                    let upgrade = future::ok(TcpStream::from(stream));
                    // TODO Change URL to Custom
                    let remote_addr = Url::parse(&format!(
                        "{}://{}:{}",
                        this.listener_addr.scheme(),
                        remote_addr.ip(),
                        remote_addr.port()
                    ))
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))
                    .unwrap();
                    let event = ListenerEvent::Incoming {
                        local_addr: this.listener_addr.clone(),
                        remote_addr,
                        upgrade,
                    };
                    Poll::Ready(event)
                }
                Poll::Ready(Err(e)) => {
                    let event = ListenerEvent::Error(e);
                    Poll::Ready(event)
                }
                Poll::Pending => return Poll::Pending,
            },
            State::Closed => Poll::Ready(ListenerEvent::Closed(Ok(()))),
        }
    }
}

// impl Stream for ListenStream {
//     type Item = ListenerEvent<Ready<Result<TcpStream, io::Error>>, io::Error>;

//     fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
//         if let Some(event) = self.pending_event.take() {
//             return Poll::Ready(Some(event));
//         }
//         tracing::trace!(
//             "ListenStream::poll_next: Polling for new connections on {}",
//             self.listener_addr
//         );
//         match Pin::new(&mut self.listener).poll_accept(cx) {
//             Poll::Ready(Ok((stream, remote_addr))) => {
//                 return Poll::Ready(Some(ListenerEvent::Incoming {
//                     local_addr: self.listener_addr,
//                     remote_addr,
//                     upgrade: future::ok(stream.into()),
//                 }));
//             }
//             Poll::Ready(Err(e)) => {
//                 return Poll::Ready(Some(ListenerEvent::Error(e)));
//             }
//             Poll::Pending => {}
//         }
//         Poll::Pending
//     }
// }
