pub mod client;
pub mod server;

use std::convert::Infallible;

use futures::future::{Ready, ready};
use volans_core::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};
use volans_swarm::{StreamProtocol, Substream};

pub struct Upgrade {
    pub(crate) supported_protocols: Vec<StreamProtocol>,
}

impl UpgradeInfo for Upgrade {
    type Info = StreamProtocol;

    type InfoIter = std::vec::IntoIter<StreamProtocol>;

    fn protocol_info(&self) -> Self::InfoIter {
        self.supported_protocols.clone().into_iter()
    }
}

impl InboundUpgrade<Substream> for Upgrade {
    type Output = (Substream, StreamProtocol);

    type Error = Infallible;

    type Future = Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: Substream, info: Self::Info) -> Self::Future {
        ready(Ok((socket, info)))
    }
}

impl OutboundUpgrade<Substream> for Upgrade {
    type Output = (Substream, StreamProtocol);

    type Error = Infallible;

    type Future = Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: Substream, info: Self::Info) -> Self::Future {
        ready(Ok((socket, info)))
    }
}
