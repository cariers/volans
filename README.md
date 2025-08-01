# Volans 网络框架
参考了libp2p，在其基础上改动整个出入站处理，去掉和p2p相应的代码。将服务端精间为只能处理入站及入站子流，也就是说不能进行对客户端发起子流。保持单向性比较符合常规开发，更容易理解和网络问题排查。

## 仓库结构

主要组件结构
 * `volans-core` 主要的trait `InboundUpgrade` `OutboundUpgrade` `Transport` `StreamMuxer` 及通用实现
 
 * `transports/` 基于`Tokio`实现了传输层`websocket` `tcp`

 * `muxers/` 实现了 `yamux` 及基于yamux精简的 [`muxing`](https://crates.io/crates/muxing) 

 * `protocols/` 目录下，实现了 `ping`

 * `volans-swarm` 实现了基于 `client` 及 `server` 的事件驱动逻辑，服务端只能接受入站连接及入站子流，相应的 客户端只能处理出站连接及出站子流

 * `examples/` 有个WebSocket的Demo