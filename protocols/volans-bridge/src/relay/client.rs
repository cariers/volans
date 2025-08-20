/// 客户端启动中继连接
/// 1、接收client 的中继请求
/// 2、建议 Src -> Dst 的连接
/// 3、通知 Dst 的 Relay Client 发送 OutboundRequest
/// 4、绑定 Src Stream 和 Dst Stream
mod behavior;
mod handler;

pub use behavior::Behavior;
