use std::{
    convert::Infallible,
    fmt, io, mem,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll},
};

use futures::{FutureExt, Stream, channel::oneshot};
use volans_core::{
    Listener, ListenerEvent, PeerId, Url, muxing::StreamMuxerBox, transport::BoxedListener,
};

static NEXT_LISTENER_ID: AtomicUsize = AtomicUsize::new(1);

/// 监听 ID，为每个监听器分配一个唯一的 ID
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct ListenerId(usize);

impl ListenerId {
    /// Creates a new `ListenerId`.
    pub fn next() -> Self {
        ListenerId(NEXT_LISTENER_ID.fetch_add(1, Ordering::SeqCst))
    }
}

impl fmt::Display for ListenerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub(crate) struct TaggedListener {
    pub(crate) id: ListenerId,
    state: ListenerState,
}

enum ListenerState {
    Active {
        listener: BoxedListener<(PeerId, StreamMuxerBox)>,
        close_rx: oneshot::Receiver<Infallible>,
    },
    Closing {
        listener: BoxedListener<(PeerId, StreamMuxerBox)>,
    },
    Done,
}

impl Unpin for TaggedListener {}

impl TaggedListener {
    pub(crate) fn new(
        id: ListenerId,
        listener: BoxedListener<(PeerId, StreamMuxerBox)>,
        close_rx: oneshot::Receiver<Infallible>,
    ) -> Self {
        TaggedListener {
            id,
            state: ListenerState::Active { listener, close_rx },
        }
    }
}

type BoxListenerUpgrade<O> = Pin<Box<dyn Future<Output = io::Result<O>> + Send>>;

pub(crate) type BoxedListenerEvent =
    ListenerEvent<BoxListenerUpgrade<(PeerId, StreamMuxerBox)>, io::Error>;

impl Stream for TaggedListener {
    type Item = (ListenerId, BoxedListenerEvent);

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            match mem::replace(&mut this.state, ListenerState::Done) {
                ListenerState::Active {
                    mut listener,
                    mut close_rx,
                } => {
                    match close_rx.poll_unpin(cx) {
                        Poll::Ready(Ok(_)) => unreachable!("Close receiver should never complete"),
                        Poll::Ready(Err(oneshot::Canceled)) => {
                            // 外部取消了监听， 切换到 Closing 状态
                            this.state = ListenerState::Closing { listener };
                            continue;
                        }
                        Poll::Pending => {}
                    }
                    match Pin::new(&mut listener).poll_event(cx) {
                        Poll::Ready(event) => {
                            this.state = ListenerState::Active { listener, close_rx };
                            // 监听器有新事件，返回事件
                            return Poll::Ready(Some((this.id, event)));
                        }
                        Poll::Pending => {
                            // 监听器还没有事件，保持状态
                            this.state = ListenerState::Active { listener, close_rx };
                        }
                    }
                }
                ListenerState::Closing { mut listener } => {
                    match Pin::new(&mut listener).poll_close(cx) {
                        Poll::Ready(res) => {
                            this.state = ListenerState::Done;
                            return Poll::Ready(Some((this.id, ListenerEvent::Closed(res))));
                        }
                        Poll::Pending => {
                            this.state = ListenerState::Closing { listener };
                        }
                    }
                }
                ListenerState::Done => return Poll::Ready(None),
            }
            return Poll::Pending;
        }
    }
}

#[derive(Debug)]
pub struct ListenOpts {
    id: ListenerId,
    addr: Url,
}

impl ListenOpts {
    pub fn new(addr: Url) -> ListenOpts {
        ListenOpts {
            id: ListenerId::next(),
            addr,
        }
    }

    pub fn listener_id(&self) -> ListenerId {
        self.id
    }

    pub fn addr(&self) -> &Url {
        &self.addr
    }
}
