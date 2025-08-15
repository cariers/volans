use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

use futures::channel::mpsc;
use parking_lot::{Mutex, MutexGuard};
use volans_core::PeerId;
use volans_swarm::{ConnectionId, StreamProtocol, Substream};

use crate::server::{AlreadyRegistered, IncomingStreams};

pub(crate) struct Shared {
    supported_protocols: HashMap<StreamProtocol, mpsc::Sender<(PeerId, ConnectionId, Substream)>>,
}

impl Shared {
    pub(crate) fn new() -> Self {
        let supported_protocols = HashMap::new();
        Self {
            supported_protocols,
        }
    }

    pub(crate) fn lock(shared: &Arc<Mutex<Self>>) -> MutexGuard<'_, Self> {
        shared.lock()
    }

    pub(crate) fn supported_protocols(&mut self) -> Vec<StreamProtocol> {
        self.supported_protocols
            .retain(|_, sender| !sender.is_closed());

        self.supported_protocols.keys().cloned().collect()
    }

    pub(crate) fn accept(
        &mut self,
        protocol: StreamProtocol,
    ) -> Result<IncomingStreams, AlreadyRegistered> {
        if self.supported_protocols.contains_key(&protocol) {
            return Err(AlreadyRegistered);
        }
        let (sender, receiver) = mpsc::channel(0);
        self.supported_protocols.insert(protocol.clone(), sender);

        Ok(IncomingStreams::new(receiver))
    }

    pub(crate) fn on_inbound_stream(
        &mut self,
        remote: PeerId,
        connection_id: ConnectionId,
        stream: Substream,
        protocol: StreamProtocol,
    ) {
        match self.supported_protocols.entry(protocol.clone()) {
            Entry::Occupied(mut entry) => {
                match entry.get_mut().try_send((remote, connection_id, stream)) {
                    Ok(()) => {}
                    Err(e) if e.is_full() => {
                        tracing::debug!(%protocol, "Channel is full, dropping inbound stream");
                    }
                    Err(e) if e.is_disconnected() => {
                        tracing::debug!(%protocol, "Channel is gone, dropping inbound stream");
                        entry.remove();
                    }
                    _ => unreachable!(),
                }
            }
            Entry::Vacant(_) => {
                tracing::debug!(%protocol, "channel is gone, dropping inbound stream");
            }
        }
    }
}
