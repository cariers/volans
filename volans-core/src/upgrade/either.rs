use std::iter::Map;

use either::Either;
use futures::future;

use crate::{
    either::EitherFuture,
    upgrade::{InboundUpgrade, OutboundUpgrade, UpgradeInfo},
};

impl<A, B> UpgradeInfo for Either<A, B>
where
    A: UpgradeInfo,
    B: UpgradeInfo,
{
    type Info = Either<A::Info, B::Info>;
    type InfoIter = Either<
        Map<<A::InfoIter as IntoIterator>::IntoIter, fn(A::Info) -> Self::Info>,
        Map<<B::InfoIter as IntoIterator>::IntoIter, fn(B::Info) -> Self::Info>,
    >;

    fn protocol_info(&self) -> Self::InfoIter {
        match self {
            Either::Left(a) => Either::Left(a.protocol_info().into_iter().map(Either::Left)),
            Either::Right(b) => Either::Right(b.protocol_info().into_iter().map(Either::Right)),
        }
    }
}

impl<C, A, B, TA, TB, EA, EB> InboundUpgrade<C> for Either<A, B>
where
    A: InboundUpgrade<C, Output = TA, Error = EA>,
    B: InboundUpgrade<C, Output = TB, Error = EB>,
{
    type Output = future::Either<TA, TB>;
    type Error = Either<EA, EB>;
    type Future = EitherFuture<A::Future, B::Future>;

    fn upgrade_inbound(self, sock: C, info: Self::Info) -> Self::Future {
        match (self, info) {
            (Either::Left(a), Either::Left(info)) => {
                EitherFuture::First(a.upgrade_inbound(sock, info))
            }
            (Either::Right(b), Either::Right(info)) => {
                EitherFuture::Second(b.upgrade_inbound(sock, info))
            }
            _ => panic!("Invalid invocation of EitherUpgrade::upgrade_inbound"),
        }
    }
}

impl<C, A, B, TA, TB, EA, EB> OutboundUpgrade<C> for Either<A, B>
where
    A: OutboundUpgrade<C, Output = TA, Error = EA>,
    B: OutboundUpgrade<C, Output = TB, Error = EB>,
{
    type Output = future::Either<TA, TB>;
    type Error = Either<EA, EB>;
    type Future = EitherFuture<A::Future, B::Future>;

    fn upgrade_outbound(self, sock: C, info: Self::Info) -> Self::Future {
        match (self, info) {
            (Either::Left(a), Either::Left(info)) => {
                EitherFuture::First(a.upgrade_outbound(sock, info))
            }
            (Either::Right(b), Either::Right(info)) => {
                EitherFuture::Second(b.upgrade_outbound(sock, info))
            }
            _ => panic!("Invalid invocation of EitherUpgrade::upgrade_outbound"),
        }
    }
}
