use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    io,
    sync::Arc,
};

use futures::channel::mpsc;
use parking_lot::{Mutex, MutexGuard};
use volans_core::PeerId;
use volans_swarm::{ConnectionId, error::DialError};

use crate::client::{StreamError, handler::NewStream};

pub(crate) struct Shared {
    connections: HashMap<PeerId, HashSet<ConnectionId>>,
    senders: HashMap<ConnectionId, mpsc::Sender<NewStream>>,
    pending_channels: HashMap<PeerId, (mpsc::Sender<NewStream>, mpsc::Receiver<NewStream>)>,
    dial_sender: mpsc::Sender<PeerId>,
}

impl Shared {
    pub(crate) fn new(dial_sender: mpsc::Sender<PeerId>) -> Self {
        Self {
            connections: HashMap::new(),
            senders: HashMap::new(),
            pending_channels: HashMap::new(),
            dial_sender,
        }
    }

    pub(crate) fn lock(shared: &Arc<Mutex<Self>>) -> MutexGuard<'_, Self> {
        shared.lock()
    }

    pub(crate) fn on_connection_established(&mut self, peer_id: PeerId, conn_id: ConnectionId) {
        self.connections.entry(peer_id).or_default().insert(conn_id);
    }

    pub(crate) fn on_connection_closed(&mut self, peer_id: PeerId, conn_id: ConnectionId) {
        match self.connections.entry(peer_id.clone()) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().remove(&conn_id);
                if entry.get().is_empty() {
                    entry.remove();
                }
            }
            Entry::Vacant(_) => {}
        }
    }

    pub(crate) fn on_dial_failure(&mut self, peer_id: PeerId, error: &DialError) {
        let Some((_, mut receiver)) = self.pending_channels.remove(&peer_id) else {
            return;
        };
        while let Ok(Some(request)) = receiver.try_next() {
            let _ = request.sender.send(Err(StreamError::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                error.to_string(),
            ))));
        }
    }

    pub(crate) fn sender(&mut self, peer: PeerId) -> mpsc::Sender<NewStream> {
        // TODO! 增加选择逻辑（最小、最后、随机、轮询），选择一个连接的 sender
        let maybe_sender = self
            .connections
            .get_mut(&peer)
            .and_then(|conns| conns.iter().next())
            .and_then(|i| self.senders.get(i));

        match maybe_sender {
            Some(sender) => sender.clone(),
            None => {
                let (sender, _) = self
                    .pending_channels
                    .entry(peer)
                    .or_insert_with(|| mpsc::channel(0));
                let _ = self.dial_sender.try_send(peer);
                sender.clone()
            }
        }
    }

    pub(crate) fn receiver(
        &mut self,
        peer: PeerId,
        connection_id: ConnectionId,
    ) -> mpsc::Receiver<NewStream> {
        if let Some((sender, receiver)) = self.pending_channels.remove(&peer) {
            tracing::debug!(%peer, %connection_id, "Returning existing pending receiver");
            self.senders.insert(connection_id, sender);
            return receiver;
        }
        tracing::debug!(%peer, %connection_id, "Creating new channel pair");
        let (sender, receiver) = mpsc::channel(0);
        self.senders.insert(connection_id, sender);
        receiver
    }
}
