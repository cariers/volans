/// 客户端启动中继连接
/// 1、swarm.dial("/ip4/xxx/peer/peer-relay-server/circuit/remote-addr")
/// 2、Transport 收到中继请求。 向Behavior发送 DialOpts
/// 3、Behavior 处理 DialOpts，向 PeerRelayServer 发送OutboundRequest
/// 4、OutboundRequest 协商成功后，通知Transport 建立连接(Substream -> Connection)
mod behavior;
mod handler;

pub use behavior::Behavior;

use crate::transport;

pub fn new() -> (transport::Config, Behavior) {
    let (transport, transport_receiver) = transport::Config::new();
    let behavior = Behavior::new(transport_receiver);
    (transport, behavior)
}
