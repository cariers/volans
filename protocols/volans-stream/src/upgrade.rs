mod codec;

pub use codec::{FramedUpgrade, WithCodecFactory};

use std::{
    convert::Infallible,
    fmt,
    future::{Ready, ready},
};

use volans_core::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};
use volans_swarm::{
    InboundUpgradeSend, OutboundUpgradeSend, StreamProtocol, Substream, SubstreamProtocol,
};

pub struct ReadyUpgrade {
    pub(crate) protocols: Vec<StreamProtocol>,
}

impl UpgradeInfo for ReadyUpgrade {
    type Info = StreamProtocol;

    type InfoIter = std::vec::IntoIter<StreamProtocol>;

    fn protocol_info(&self) -> Self::InfoIter {
        self.protocols.clone().into_iter()
    }
}

impl InboundUpgrade<Substream> for ReadyUpgrade {
    type Output = (Substream, StreamProtocol);
    type Error = Infallible;

    type Future = Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: Substream, info: Self::Info) -> Self::Future {
        ready(Ok((socket, info)))
    }
}

impl OutboundUpgrade<Substream> for ReadyUpgrade {
    type Output = (Substream, StreamProtocol);
    type Error = Infallible;

    type Future = Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: Substream, info: Self::Info) -> Self::Future {
        ready(Ok((socket, info)))
    }
}

pub trait InboundStreamUpgradeFactory: Send + 'static {
    type Output: Send + 'static;
    type Error: Send + fmt::Debug + 'static;
    type Upgrade: InboundUpgradeSend<
            Info = StreamProtocol,
            Output = (Self::Output, StreamProtocol),
            Error = Self::Error,
        >;
    fn listen_protocol(
        &self,
        protocols: Vec<StreamProtocol>,
    ) -> SubstreamProtocol<Self::Upgrade, ()>;
}

pub trait OutboundStreamUpgradeFactory: Send + 'static {
    type Output: Send + 'static;
    type Error: Send + fmt::Debug + 'static;
    type Upgrade: OutboundUpgradeSend<
            Info = StreamProtocol,
            Output = (Self::Output, StreamProtocol),
            Error = Self::Error,
        >;
    fn outbound_request(&self, protocol: StreamProtocol) -> SubstreamProtocol<Self::Upgrade, ()>;
}

#[derive(Clone)]
pub struct ReadyUpgradeFactory;

impl InboundStreamUpgradeFactory for ReadyUpgradeFactory {
    type Output = Substream;
    type Error = Infallible;
    type Upgrade = ReadyUpgrade;

    fn listen_protocol(
        &self,
        protocols: Vec<StreamProtocol>,
    ) -> SubstreamProtocol<Self::Upgrade, ()> {
        SubstreamProtocol::new(ReadyUpgrade { protocols }, ())
    }
}

impl OutboundStreamUpgradeFactory for ReadyUpgradeFactory {
    type Output = Substream;
    type Error = Infallible;
    type Upgrade = ReadyUpgrade;

    fn outbound_request(&self, protocol: StreamProtocol) -> SubstreamProtocol<Self::Upgrade, ()> {
        SubstreamProtocol::new(
            ReadyUpgrade {
                protocols: vec![protocol],
            },
            (),
        )
    }
}
