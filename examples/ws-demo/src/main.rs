use std::{collections::HashMap, pin::Pin, time::Duration};

use futures::StreamExt;
use mdns_sd::{IfKind, ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::time::sleep;
use volans::{
    Transport,
    core::{Multiaddr, PeerId, identity::KeyPair, multiaddr::Protocol},
    muxing, plaintext,
    swarm::{self, NetworkIncomingBehavior, NetworkOutgoingBehavior},
    ws,
};

#[derive(Default, Debug, Clone, Copy)]
pub struct TokioExecutor;

impl swarm::Executor for TokioExecutor {
    fn exec(&self, future: Pin<Box<dyn Future<Output = ()> + Send>>) {
        tokio::spawn(future);
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("Starting WebSocket Demo");

    let service_daemon = ServiceDaemon::new().expect("Failed to create mDNS service daemon");
    let mdns_fut = start_mdns(service_daemon.clone());
    tokio::spawn(async move {
        if let Err(e) = mdns_fut.await {
            tracing::error!("mDNS error: {:?}", e);
        }
    });
    let signal_c = tokio::signal::ctrl_c();
    tokio::select! {
        _ = start_demo() => {
            tracing::info!("Demo completed successfully");
        }
        _ = signal_c => {
            tracing::info!("Received Ctrl+C signal");
        }
    }
    robust_shutdown(service_daemon, 3)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to shutdown gracefully: {:?}", e);
        });
    Ok(())
}

async fn robust_shutdown(daemon: ServiceDaemon, max_retries: u32) -> Result<(), mdns_sd::Error> {
    for attempt in 1..=max_retries {
        match daemon.shutdown() {
            Ok(receiver) => {
                // 尝试接收关闭确认
                match receiver.recv_timeout(Duration::from_secs(5)) {
                    Ok(mdns_sd::DaemonStatus::Shutdown) => {
                        println!("Shutdown completed on attempt {}", attempt);
                        return Ok(());
                    }
                    Ok(other) => {
                        println!("Attempt {}: Unexpected status {:?}", attempt, other);
                    }
                    Err(e) => {
                        println!("Attempt {}: Timeout or error: {}", attempt, e);
                    }
                }
            }
            Err(mdns_sd::Error::Again) => {
                // 按照文档建议，Error::Again 时应该重试
                println!("Attempt {}: Got Error::Again, retrying...", attempt);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => {
                println!("Attempt {}: Fatal error: {}", attempt, e);
                return Err(e);
            }
        }

        if attempt < max_retries {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    Err(mdns_sd::Error::Msg(
        "All shutdown attempts failed".to_string(),
    ))
}

async fn start_mdns(service_daemon: ServiceDaemon) -> anyhow::Result<()> {
    service_daemon.enable_interface(IfKind::IPv4).unwrap();
    service_daemon.set_multicast_loop_v6(false).unwrap();

    let client_daemon = service_daemon.clone();
    tokio::spawn(async move {
        sleep(Duration::from_secs(5)).await;
        if let Err(e) = mdns_discovery(client_daemon).await {
            tracing::error!("mDNS Discovery error: {:?}", e);
        }
    });

    let monitor_daemon = service_daemon.clone();

    tokio::spawn(async move {
        while let Some(event) = monitor_daemon.monitor().unwrap().into_stream().next().await {
            tracing::info!("mDNS Monitor Event: {:?}", event);
        }
    });

    let mut txt_records = HashMap::new();
    txt_records.insert("DNS_ADDR".to_string(), "/test/dns/addr".to_string());

    let name = "FxesryGX2bq7zmPmubJ4fZVFpghULHcYA4PkwQnjx3Kc".to_string();

    let mut service_info = ServiceInfo::new(
        "_peer._udp.local.",
        &name.clone(),
        &format!("{}.local.", name),
        (),
        80,
        txt_records,
    )
    .unwrap()
    .enable_addr_auto();

    service_info.set_requires_probe(false);

    service_daemon
        .register(service_info)
        .expect("Failed to register service");

    Ok(())
}

async fn mdns_discovery(service_daemon: ServiceDaemon) -> anyhow::Result<()> {
    let mut browser_stream = service_daemon
        .browse("_peer._udp.local.")
        .unwrap()
        .into_stream();

    while let Some(event) = browser_stream.next().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                tracing::info!(
                    "Service resolved: {} at {:?}, info:{:?}",
                    info.get_fullname(),
                    info.get_properties(),
                    info
                );
            }
            event => tracing::debug!("mDNS Event: {:?}", event),
        }
    }

    Ok(())
}

async fn start_demo() -> anyhow::Result<()> {
    tracing::info!("Starting TCP Echo Example");

    tokio::spawn(async move {
        if let Err(e) = start_backend().await {
            tracing::error!("Backend Server error: {:?}", e);
        }
    });

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if let Err(e) = start_client().await {
            tracing::error!("Client error: {:?}", e);
        }
    });

    start_bridge().await?;

    Ok(())
}

//中继服务[id:1]
async fn start_bridge() -> anyhow::Result<()> {
    tracing::info!("Starting Bridge");

    let mut bytes = [0u8; 32];
    bytes[0] = 1;

    let key_pair = KeyPair::from_bytes(&bytes);

    let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    tracing::info!("Bridge Local Peer ID: {:?}", local_peer_id);

    let addr = "/ip4/0.0.0.0/tcp/8088/ws".parse::<Multiaddr>()?;

    let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

    let muxing_upgrade = muxing::Config::new();

    let (bridge_server_behavior, bridge_client_behavior) = volans::bridge::relay::new();

    // 对外服务
    let transport_client = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade.clone())
        .multiplex(muxing_upgrade.clone())
        .boxed();

    let client_behavior = BridgeClientBehavior {
        // ping: volans::ping::outbound::Behavior::default(),
        bridge: bridge_client_behavior,
    };

    let mut swarm_client = swarm::client::Swarm::new(
        transport_client,
        client_behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    // 对外服务
    let transport_server = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade.clone())
        .multiplex(muxing_upgrade.clone())
        .boxed();

    let server_behavior = BridgeServerBehavior {
        // ping: volans::ping::inbound::Behavior::default(),
        bridge: bridge_server_behavior,
    };

    let mut swarm_server = swarm::server::Swarm::new(
        transport_server,
        server_behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    tokio::spawn(async move {
        while let Some(event) = swarm_client.next().await {
            tracing::info!("BridgeClientSwarm: {:?}", event);
        }
    });

    swarm_server.listen_on(addr.clone())?;

    while let Some(event) = swarm_server.next().await {
        tracing::info!("BridgeServerSwarm: {:?}", event);
    }

    Ok(())
}

#[derive(NetworkIncomingBehavior)]
struct BridgeServerBehavior {
    // ping: volans::ping::inbound::Behavior,
    bridge: volans::bridge::relay::server::Behavior,
}

#[derive(NetworkOutgoingBehavior)]
struct BridgeClientBehavior {
    // ping: volans::ping::outbound::Behavior,
    bridge: volans::bridge::relay::client::Behavior,
}

//后端服务[id:2]
async fn start_backend() -> anyhow::Result<()> {
    tracing::info!("Starting Backend");
    let mut bytes = [0u8; 32];
    bytes[0] = 2;

    let key_pair = KeyPair::from_bytes(&bytes);
    let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    tracing::info!("Backend Local Peer ID: {:?}", local_peer_id);

    let addr = "/ip4/127.0.0.1/tcp/8089/ws".parse::<Multiaddr>()?;

    let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

    let muxing_upgrade = muxing::Config::new();

    // 对外服务
    let direct_transport = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade.clone())
        .multiplex(muxing_upgrade.clone())
        .boxed();

    let (bridge_transport, bridge_behavior) = volans::bridge::backend::new();

    let transport = bridge_transport
        .upgrade()
        .authenticate(identify_upgrade)
        .multiplex(muxing_upgrade)
        .boxed()
        .choice(direct_transport)
        .boxed()
        .map(|e, _| e.into_inner())
        .boxed();
    // .map(|e, _| e.into_inner());

    let behavior = BackendServerBehavior {
        // ping: volans::ping::inbound::Behavior::default(),
        bridge: bridge_behavior,
    };

    let mut swarm = swarm::server::Swarm::new(
        transport,
        behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    let _ = swarm.listen_on(addr.clone())?;
    let _ = swarm.listen_on(Protocol::Circuit.into());

    while let Some(event) = swarm.next().await {
        tracing::info!("BackendServerSwarm: {:?}", event);
    }

    Ok(())
}

#[derive(NetworkIncomingBehavior)]
struct BackendServerBehavior {
    // ping: volans::ping::inbound::Behavior,
    bridge: volans::bridge::backend::Behavior,
}

// 启动客户端[id:3]
async fn start_client() -> anyhow::Result<()> {
    tracing::info!("Starting TCP Demo Client");

    let addr = "/ip4/127.0.0.1/tcp/8088/ws".parse::<Multiaddr>()?;

    let mut bytes = [0u8; 32];
    bytes[0] = 3;

    let key_pair = KeyPair::from_bytes(&bytes);

    // let key: [u8; 32] = rand::random();
    // let local_key = PublicKey::from_bytes(&key);
    let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    tracing::info!("Client Local Peer ID: {:?}", local_peer_id);

    let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

    let muxing_upgrade = muxing::Config::new();

    // 对外服务
    let direct_transport = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade.clone())
        .multiplex(muxing_upgrade.clone())
        .boxed();

    let (bridge_transport, bridge_behavior) = volans::bridge::client::new();

    let transport = bridge_transport
        .upgrade()
        .authenticate(identify_upgrade)
        .multiplex(muxing_upgrade)
        .boxed()
        .choice(direct_transport)
        .boxed()
        .map(|e, _| e.into_inner())
        .boxed();

    let behavior = ClientOutboundBehavior {
        // ping: volans::ping::outbound::Behavior::default(),
        bridge: bridge_behavior,
    };

    let mut swarm = swarm::client::Swarm::new(
        transport,
        behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    let mut bytes = [0u8; 32];
    bytes[0] = 1;
    let key_pair = KeyPair::from_bytes(&bytes);
    let bridge_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    bytes[0] = 2;

    let key_pair = KeyPair::from_bytes(&bytes);
    let backend_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    // let _ = swarm
    //     .dial(swarm::DialOpts::new(Some(addr.clone()), None))
    //     .unwrap();

    let backend_addr = addr
        .with(Protocol::Peer(bridge_peer_id))
        .with(Protocol::Circuit)
        .with(Protocol::Peer(backend_peer_id));

    tracing::info!("Client Starting dial Backend: {}", backend_addr);

    let _ = swarm
        .dial(swarm::DialOpts::new(Some(backend_addr), None))
        .unwrap();

    while let Some(event) = swarm.next().await {
        tracing::info!("Client Swarm event: {:?}", event);
    }
    Ok(())
}

#[derive(NetworkOutgoingBehavior)]
struct ClientOutboundBehavior {
    // ping: volans::ping::outbound::Behavior,
    bridge: volans::bridge::client::Behavior,
}
