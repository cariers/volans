use std::io;

use futures::{AsyncRead, AsyncWrite, future};
use volans_codec::asynchronous_codec::{Decoder, Encoder, Framed};
use volans_core::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};
use volans_swarm::{StreamProtocol, Substream, SubstreamProtocol};

use crate::{InboundStreamUpgradeFactory, OutboundStreamUpgradeFactory};

pub struct WithCodecFactory<TCodec> {
    codec: TCodec,
}

impl<TCodec> WithCodecFactory<TCodec> {
    pub fn new(codec: TCodec) -> Self {
        Self { codec }
    }

    pub fn codec(&self) -> &TCodec {
        &self.codec
    }

    pub fn codec_mut(&mut self) -> &mut TCodec {
        &mut self.codec
    }
}

impl<TCodec> InboundStreamUpgradeFactory for WithCodecFactory<TCodec>
where
    TCodec: Decoder + Encoder + Clone + Send + 'static,
{
    type Output = Framed<Substream, TCodec>;
    type Error = io::Error;
    type Upgrade = FramedUpgrade<TCodec>;

    fn listen_protocol(
        &self,
        protocols: Vec<StreamProtocol>,
    ) -> SubstreamProtocol<Self::Upgrade, ()> {
        SubstreamProtocol::new(
            FramedUpgrade {
                protocols,
                codec: self.codec.clone(),
            },
            (),
        )
    }
}

impl<TCodec> OutboundStreamUpgradeFactory for WithCodecFactory<TCodec>
where
    TCodec: Decoder + Encoder + Clone + Send + 'static,
{
    type Output = Framed<Substream, TCodec>;
    type Error = io::Error;
    type Upgrade = FramedUpgrade<TCodec>;

    fn outbound_request(&self, protocol: StreamProtocol) -> SubstreamProtocol<Self::Upgrade, ()> {
        SubstreamProtocol::new(
            FramedUpgrade {
                protocols: vec![protocol],
                codec: self.codec.clone(),
            },
            (),
        )
    }
}

pub struct FramedUpgrade<TCodec> {
    pub(crate) protocols: Vec<StreamProtocol>,
    codec: TCodec,
}

impl<TCodec> UpgradeInfo for FramedUpgrade<TCodec> {
    type Info = StreamProtocol;
    type InfoIter = std::vec::IntoIter<StreamProtocol>;

    fn protocol_info(&self) -> Self::InfoIter {
        self.protocols.clone().into_iter()
    }
}

impl<TCodec, C> InboundUpgrade<C> for FramedUpgrade<TCodec>
where
    TCodec: Decoder + Encoder + Clone,
    C: AsyncWrite + AsyncRead,
{
    type Output = (Framed<C, TCodec>, StreamProtocol);
    type Error = io::Error;

    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future {
        let framed = Framed::new(socket, self.codec.clone());
        future::ready(Ok((framed, info)))
    }
}

impl<TCodec, C> OutboundUpgrade<C> for FramedUpgrade<TCodec>
where
    TCodec: Decoder + Encoder + Clone,
    C: AsyncWrite + AsyncRead,
{
    type Output = (Framed<C, TCodec>, StreamProtocol);
    type Error = io::Error;

    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future {
        let framed = Framed::new(socket, self.codec.clone());
        future::ready(Ok((framed, info)))
    }
}
