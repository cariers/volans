use std::{convert::Infallible, iter};

use futures::future;

use crate::upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};

#[derive(Debug, Copy, Clone)]
pub struct DeniedUpgrade;

impl UpgradeInfo for DeniedUpgrade {
    type Info = &'static str;
    type InfoIter = iter::Empty<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::empty()
    }
}

impl<C> InboundUpgrade<C> for DeniedUpgrade {
    type Output = Infallible;
    type Error = Infallible;
    type Future = future::Pending<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, _: C, _: Self::Info) -> Self::Future {
        future::pending()
    }
}

impl<C> OutboundUpgrade<C> for DeniedUpgrade {
    type Output = Infallible;
    type Error = Infallible;
    type Future = future::Pending<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, _: C, _: Self::Info) -> Self::Future {
        future::pending()
    }
}
