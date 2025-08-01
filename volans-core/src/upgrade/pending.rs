use std::{convert::Infallible, iter};

use futures::future;

use crate::upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};

#[derive(Debug, Copy, Clone)]
pub struct PendingUpgrade<P> {
    protocol_name: P,
}

impl<P> PendingUpgrade<P> {
    pub fn new(protocol_name: P) -> Self {
        Self { protocol_name }
    }
}

impl<P> UpgradeInfo for PendingUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Info = P;
    type InfoIter = iter::Once<P>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(self.protocol_name.clone())
    }
}

impl<C, P> InboundUpgrade<C> for PendingUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = Infallible;
    type Error = Infallible;
    type Future = future::Pending<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, _: C, _: Self::Info) -> Self::Future {
        future::pending()
    }
}

impl<C, P> OutboundUpgrade<C> for PendingUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = Infallible;
    type Error = Infallible;
    type Future = future::Pending<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, _: C, _: Self::Info) -> Self::Future {
        future::pending()
    }
}
