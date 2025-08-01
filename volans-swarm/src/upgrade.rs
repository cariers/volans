use volans_core::upgrade;

use crate::Substream;

pub trait UpgradeInfoSend: Send + 'static {
    type Info: AsRef<str> + Clone + Send + 'static;
    type InfoIter: Iterator<Item = Self::Info> + Send + 'static;

    fn protocol_info(&self) -> Self::InfoIter;
}

impl<T> UpgradeInfoSend for T
where
    T: upgrade::UpgradeInfo + Send + 'static,
    T::Info: Send + 'static,
    <T::InfoIter as IntoIterator>::IntoIter: Send + 'static,
{
    type Info = T::Info;
    type InfoIter = <T::InfoIter as IntoIterator>::IntoIter;

    fn protocol_info(&self) -> Self::InfoIter {
        upgrade::UpgradeInfo::protocol_info(self).into_iter()
    }
}

pub trait InboundUpgradeSend: UpgradeInfoSend {
    type Output: Send + 'static;
    type Error: Send + 'static;
    type Future: Future<Output = Result<Self::Output, Self::Error>> + Send + 'static;

    fn upgrade_inbound(self, socket: Substream, info: Self::Info) -> Self::Future;
}

impl<T, TInfo> InboundUpgradeSend for T
where
    T: upgrade::InboundUpgrade<Substream, Info = TInfo> + UpgradeInfoSend<Info = TInfo>,
    TInfo: AsRef<str> + Clone + Send + 'static,
    T::Output: Send + 'static,
    T::Error: Send + 'static,
    T::Future: Send + 'static,
{
    type Output = T::Output;
    type Error = T::Error;
    type Future = T::Future;

    fn upgrade_inbound(self, socket: Substream, info: TInfo) -> Self::Future {
        upgrade::InboundUpgrade::upgrade_inbound(self, socket, info)
    }
}

pub trait OutboundUpgradeSend: UpgradeInfoSend {
    type Output: Send + 'static;
    type Error: Send + 'static;
    type Future: Future<Output = Result<Self::Output, Self::Error>> + Send + 'static;

    fn upgrade_outbound(self, socket: Substream, info: Self::Info) -> Self::Future;
}

impl<T, TInfo> OutboundUpgradeSend for T
where
    T: upgrade::OutboundUpgrade<Substream, Info = TInfo> + UpgradeInfoSend<Info = TInfo>,
    TInfo: AsRef<str> + Clone + Send + 'static,
    T::Output: Send + 'static,
    T::Error: Send + 'static,
    T::Future: Send + 'static,
{
    type Output = T::Output;
    type Error = T::Error;
    type Future = T::Future;

    fn upgrade_outbound(self, socket: Substream, info: TInfo) -> Self::Future {
        upgrade::OutboundUpgrade::upgrade_outbound(self, socket, info)
    }
}

pub struct SendWrapper<T>(pub T);

impl<T: UpgradeInfoSend> upgrade::UpgradeInfo for SendWrapper<T> {
    type Info = T::Info;
    type InfoIter = T::InfoIter;

    fn protocol_info(&self) -> Self::InfoIter {
        UpgradeInfoSend::protocol_info(&self.0)
    }
}

impl<T: OutboundUpgradeSend> upgrade::OutboundUpgrade<Substream> for SendWrapper<T> {
    type Output = T::Output;
    type Error = T::Error;
    type Future = T::Future;

    fn upgrade_outbound(self, socket: Substream, info: T::Info) -> Self::Future {
        OutboundUpgradeSend::upgrade_outbound(self.0, socket, info)
    }
}

impl<T: InboundUpgradeSend> upgrade::InboundUpgrade<Substream> for SendWrapper<T> {
    type Output = T::Output;
    type Error = T::Error;
    type Future = T::Future;

    fn upgrade_inbound(self, socket: Substream, info: T::Info) -> Self::Future {
        InboundUpgradeSend::upgrade_inbound(self.0, socket, info)
    }
}
