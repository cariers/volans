use std::pin::Pin;

use futures::StreamExt;
use volans::{
    Transport,
    core::{Multiaddr, PeerId, identity::KeyPair},
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

#[derive(NetworkIncomingBehavior)]
struct GatewayInboundBehavior {
    ping: volans::ping::inbound::Behavior,
}

#[derive(NetworkOutgoingBehavior)]
struct GatewayOutboundBehavior {
    ping: volans::ping::outbound::Behavior,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // init tracing
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    tracing::info!("Starting TCP Echo Example");

    let addr = "/ip4/0.0.0.0/tcp/8088/ws".parse::<Multiaddr>()?;

    let mut bytes = [0u8; 32];
    bytes[0] = 1;

    let key_pair = KeyPair::from_bytes(&bytes);

    // let key: [u8; 32] = rand::random();
    // let local_key = PublicKey::from_bytes(&key);
    let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

    let muxing_upgrade = muxing::Config::new();

    let transport = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade)
        .multiplex(muxing_upgrade)
        .boxed();

    let behavior = GatewayInboundBehavior {
        ping: volans::ping::inbound::Behavior::default(),
    };

    let mut swarm = swarm::server::Swarm::new(
        transport,
        behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    let _ = swarm.listen_on(addr.clone())?;

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        if let Err(e) = start_client().await {
            tracing::error!("Server error: {:?}", e);
        }
    });

    while let Some(event) = swarm.next().await {
        tracing::info!("Server Swarm event: {:?}", event);
    }
    Ok(())
}

async fn start_client() -> anyhow::Result<()> {
    tracing::info!("Starting TCP Demo Client");

    let addr = "/ip4/127.0.0.1/tcp/8088/ws".parse::<Multiaddr>()?;

    let mut bytes = [0u8; 32];
    bytes[0] = 2;

    let key_pair = KeyPair::from_bytes(&bytes);

    // let key: [u8; 32] = rand::random();
    // let local_key = PublicKey::from_bytes(&key);
    let local_peer_id = PeerId::from_public_key(&key_pair.verifying_key());

    let identify_upgrade = plaintext::Config::new(key_pair.verifying_key());

    let muxing_upgrade = muxing::Config::new();

    let transport = ws::Config::new()
        .upgrade()
        .authenticate(identify_upgrade)
        .multiplex(muxing_upgrade)
        .boxed();

    let behavior = GatewayOutboundBehavior {
        ping: volans::ping::outbound::Behavior::default(),
    };

    let mut swarm = swarm::client::Swarm::new(
        transport,
        behavior,
        local_peer_id,
        swarm::connection::PoolConfig::new(Box::new(TokioExecutor)),
    );

    let _ = swarm.dial(swarm::DialOpts::new(Some(addr), None)).unwrap();

    while let Some(event) = swarm.next().await {
        tracing::info!("Client Swarm event: {:?}", event);
    }
    Ok(())
}
