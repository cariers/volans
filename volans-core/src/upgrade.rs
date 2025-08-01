mod apply;
mod denied;
mod either;
mod error;
mod pending;
mod ready;
mod select;

pub use apply::{InboundUpgradeApply, OutboundUpgradeApply, apply};
pub use denied::DeniedUpgrade;
pub use error::UpgradeError;
pub use pending::PendingUpgrade;
pub use ready::ReadyUpgrade;
pub use select::SelectUpgrade;

/// 升级信息
pub trait UpgradeInfo {
    type Info: AsRef<str> + Clone;
    type InfoIter: Iterator<Item = Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter;
}

pub trait InboundUpgrade<C>: UpgradeInfo {
    type Output;
    type Error;
    type Future: Future<Output = Result<Self::Output, Self::Error>>;

    /// 升级入站流
    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future;
}

pub trait OutboundUpgrade<C>: UpgradeInfo {
    type Output;
    type Error;
    type Future: Future<Output = Result<Self::Output, Self::Error>>;

    /// 升级出站流
    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future;
}

pub trait InboundConnectionUpgrade<C>: UpgradeInfo {
    type Output;
    type Error;
    type Future: Future<Output = Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future;
}

pub trait OutboundConnectionUpgrade<C>: UpgradeInfo {
    type Output;
    type Error;
    type Future: Future<Output = Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future;
}
