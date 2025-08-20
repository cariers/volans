/// 客户端启动中继连接
/// 1、接收到中继请求
/// 2、查询DstPeerId
/// 3、通过 Relay Client 发起连接
/// 4、给DstPeerId Relay Client 发送OutboundRequest
/// 5、绑定 Src Stream 和 Dst Stream
mod behavior;
mod handler;

pub use behavior::Behavior;
