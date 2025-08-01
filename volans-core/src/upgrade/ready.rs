use std::{convert::Infallible, iter};

use futures::future;

use crate::upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo};

#[derive(Debug, Copy, Clone)]
pub struct ReadyUpgrade<P> {
    protocol_name: P,
}

impl<P> ReadyUpgrade<P> {
    pub const fn new(protocol_name: P) -> Self {
        Self { protocol_name }
    }
}

impl<P> UpgradeInfo for ReadyUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Info = P;
    type InfoIter = iter::Once<P>;

    fn protocol_info(&self) -> Self::InfoIter {
        iter::once(self.protocol_name.clone())
    }
}

impl<C, P> InboundUpgrade<C> for ReadyUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = C;
    type Error = Infallible;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, stream: C, _: Self::Info) -> Self::Future {
        future::ready(Ok(stream))
    }
}

impl<C, P> OutboundUpgrade<C> for ReadyUpgrade<P>
where
    P: AsRef<str> + Clone,
{
    type Output = C;
    type Error = Infallible;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, stream: C, _: Self::Info) -> Self::Future {
        future::ready(Ok(stream))
    }
}
