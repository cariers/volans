#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Ok(())
}

// use std::pin::Pin;

// use futures::StreamExt;
// use volans::{
//     Transport,
//     core::{Multiaddr, PeerId, identity::KeyPair, multiaddr::Protocol},
//     muxing, plaintext,
//     registry::{Config, MdnsRegistry},
//     swarm::{self, NetworkIncomingBehavior, NetworkOutgoingBehavior},
//     ws,
// };

// #[derive(Default, Debug, Clone, Copy)]
// pub struct TokioExecutor;

// impl swarm::Executor for TokioExecutor {
//     fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
//         tokio::spawn(future);
//     }
// }

// #[tokio::main]
// async fn main() -> anyhow::Result<()> {
//     tracing_subscriber::fmt()
//         .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
//         .with_thread_ids(true)
//         .with_file(true)
//         .with_line_number(true)
//         .init();

//     tracing::info!("Starting WebSocket Demo");

//     let signal_c = tokio::signal::ctrl_c();
//     tokio::select! {
//         _ = start_demo() => {
//             tracing::info!("Demo completed successfully");
//         }
//         _ = signal_c => {
//             tracing::info!("Received Ctrl+C signal");
//         }
//     }
//     Ok(())
// }

// async fn start_demo() -> anyhow::Result<()> {
//     tracing::info!("Starting TCP Echo Example");

//     // 获取命令行参数，执行不同的服务
//     let args: Vec<String> = std::env::args().collect();
//     if args.len() > 1 {
//         let service = &args[1];
//         match service.as_str() {
//             "backend" => start_backend().await?,
//             "client" => start_client().await?,
//             "bridge" => start_bridge().await?,
//             _ => {
//                 tracing::error!("Unknown service: {}", service);
//             }
//         }
//     }

//     Ok(())
// }

// //中继服务[id:1]
// async fn start_bridge() -> anyhow::Result<()> {
//     tracing::info!("Starting Bridge");

//     let mut bytes = [0u8; 32];
//     bytes[0] = 1;

//     let key_pair = KeyPair::from_bytes(&bytes);

//     let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

//     tracing::info!("Bridge Local Peer ID: {:?}", local_peer_id);

//     let addr = "/ip4/0.0.0.0/tcp/8088/ws".parse::<Multiaddr>()?;

//     let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

//     let muxing_upgrade = muxing::Config::new();

//     let (bridge_server_behavior, bridge_client_behavior) = volans::bridge::relay::new();

//     // 对外服务
//     let transport_client = ws::Config::new()
//         .upgrade()
//         .authenticate(identify_upgrade.clone())
//         .multiplex(muxing_upgrade.clone())
//         .boxed();

//     let registry = volans::registry::discovery::Behavior::default();

//     let client_behavior = BridgeClientBehavior {
//         ping: volans::ping::outbound::Behavior::default(),
//         bridge: bridge_client_behavior,
//         registry,
//     };

//     let mut swarm_client = swarm::client::Swarm::new(
//         transport_client,
//         client_behavior,
//         local_peer_id,
//         swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
//     );

//     // 对外服务
//     let transport_server = ws::Config::new()
//         .upgrade()
//         .authenticate(identify_upgrade.clone())
//         .multiplex(muxing_upgrade.clone())
//         .boxed();

//     let registry = volans::registry::registry::Behavior::new(
//         local_peer_id,
//         MdnsRegistry::default(),
//         Config::default(),
//     );

//     let server_behavior = BridgeServerBehavior {
//         ping: volans::ping::inbound::Behavior::default(),
//         bridge: bridge_server_behavior,
//         registry,
//     };

//     let mut swarm_server = swarm::server::Swarm::new(
//         transport_server,
//         server_behavior,
//         local_peer_id,
//         swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
//     );

//     tokio::spawn(async move {
//         while let Some(event) = swarm_client.next().await {
//             match event {
//                 swarm::client::SwarmEvent::Behavior(BridgeClientBehaviorEvent::Ping(_)) => {}
//                 _ => {
//                     tracing::info!("BridgeClientSwarm: {:?}", event);
//                 }
//             }
//         }
//     });

//     swarm_server.listen_on(addr.clone())?;

//     while let Some(event) = swarm_server.next().await {
//         match event {
//             swarm::server::SwarmEvent::Behavior(BridgeServerBehaviorEvent::Ping(_)) => {}
//             _ => {
//                 tracing::info!("BridgeServerSwarm: {:?}", event);
//             }
//         }
//     }

//     Ok(())
// }

// #[derive(NetworkIncomingBehavior)]
// struct BridgeServerBehavior {
//     ping: volans::ping::inbound::Behavior,
//     bridge: volans::bridge::relay::server::Behavior,
//     registry: volans::registry::registry::Behavior<MdnsRegistry>,
// }

// #[derive(NetworkOutgoingBehavior)]
// struct BridgeClientBehavior {
//     ping: volans::ping::outbound::Behavior,
//     bridge: volans::bridge::relay::client::Behavior,
//     registry: volans::registry::discovery::Behavior<MdnsRegistry>,
// }

// //后端服务[id:2]
// async fn start_backend() -> anyhow::Result<()> {
//     tracing::info!("Starting Backend");
//     let mut bytes = [0u8; 32];
//     bytes[0] = 2;

//     let key_pair = KeyPair::from_bytes(&bytes);
//     let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

//     tracing::info!("Backend Local Peer ID: {:?}", local_peer_id);

//     let addr = "/ip4/0.0.0.0/tcp/8089/ws".parse::<Multiaddr>()?;

//     let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

//     let muxing_upgrade = muxing::Config::new();

//     // 对外服务
//     let direct_transport = ws::Config::new()
//         .upgrade()
//         .authenticate(identify_upgrade.clone())
//         .multiplex(muxing_upgrade.clone())
//         .boxed();

//     let (bridge_transport, bridge_behavior) = volans::bridge::backend::new();

//     let transport = bridge_transport
//         .upgrade()
//         .authenticate(identify_upgrade)
//         .multiplex(muxing_upgrade)
//         .boxed()
//         .choice(direct_transport)
//         .boxed()
//         .map(|e, _| e.into_inner())
//         .boxed();
//     // .map(|e, _| e.into_inner());

//     let registry = volans::registry::registry::Behavior::new(
//         local_peer_id,
//         MdnsRegistry::default(),
//         Config::default(),
//     );

//     let behavior = BackendServerBehavior {
//         ping: volans::ping::inbound::Behavior::default(),
//         bridge: bridge_behavior,
//         registry,
//     };

//     let mut swarm = swarm::server::Swarm::new(
//         transport,
//         behavior,
//         local_peer_id,
//         swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
//     );

//     let _ = swarm.listen_on(addr.clone())?;
//     let _ = swarm.listen_on(Protocol::Circuit.into());

//     while let Some(event) = swarm.next().await {
//         match event {
//             swarm::server::SwarmEvent::Behavior(BackendServerBehaviorEvent::Ping(_)) => {}
//             _ => {
//                 tracing::info!("BackendServerSwarm: {:?}", event);
//             }
//         }
//     }

//     Ok(())
// }

// #[derive(NetworkIncomingBehavior)]
// struct BackendServerBehavior {
//     ping: volans::ping::inbound::Behavior,
//     bridge: volans::bridge::backend::Behavior,
//     registry: volans::registry::registry::Behavior<MdnsRegistry>,
// }

// // 启动客户端[id:3]
// async fn start_client() -> anyhow::Result<()> {
//     tracing::info!("Starting TCP Demo Client");

//     let addr = "/ip4/127.0.0.1/tcp/8088/ws".parse::<Multiaddr>()?;

//     let mut bytes = [0u8; 32];
//     bytes[0] = 3;

//     let key_pair = KeyPair::from_bytes(&bytes);

//     // let key: [u8; 32] = rand::random();
//     // let local_key = PublicKey::from_bytes(&key);
//     let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

//     tracing::info!("Client Local Peer ID: {:?}", local_peer_id);

//     let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

//     let muxing_upgrade = muxing::Config::new();

//     // 对外服务
//     let direct_transport = ws::Config::new()
//         .upgrade()
//         .authenticate(identify_upgrade.clone())
//         .multiplex(muxing_upgrade.clone())
//         .boxed();

//     let (bridge_transport, bridge_behavior) = volans::bridge::client::new();

//     let transport = bridge_transport
//         .upgrade()
//         .authenticate(identify_upgrade)
//         .multiplex(muxing_upgrade)
//         .boxed()
//         .choice(direct_transport)
//         .boxed()
//         .map(|e, _| e.into_inner())
//         .boxed();

//     let behavior = ClientOutboundBehavior {
//         ping: volans::ping::outbound::Behavior::default(),
//         bridge: bridge_behavior,
//     };

//     let mut swarm = swarm::client::Swarm::new(
//         transport,
//         behavior,
//         local_peer_id,
//         swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
//     );

//     let mut bytes = [0u8; 32];
//     bytes[0] = 1;
//     let key_pair = KeyPair::from_bytes(&bytes);
//     let bridge_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

//     bytes[0] = 2;

//     let key_pair = KeyPair::from_bytes(&bytes);
//     let backend_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

//     // let _ = swarm
//     //     .dial(swarm::DialOpts::new(Some(addr.clone()), None))
//     //     .unwrap();

//     let backend_addr = addr
//         .with(Protocol::Peer(bridge_peer_id))
//         .with(Protocol::Circuit)
//         .with(Protocol::Peer(backend_peer_id));

//     tracing::info!("Client Starting dial Backend: {}", backend_addr);

//     let _ = swarm
//         .dial(swarm::DialOpts::new(Some(backend_addr), None))
//         .unwrap();

//     while let Some(event) = swarm.next().await {
//         tracing::info!("Client Swarm event: {:?}", event);
//     }
//     Ok(())
// }

// #[derive(NetworkOutgoingBehavior)]
// struct ClientOutboundBehavior {
//     ping: volans::ping::outbound::Behavior,
//     bridge: volans::bridge::client::Behavior,
// }
