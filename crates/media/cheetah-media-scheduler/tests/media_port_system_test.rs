//! GB4-MED-008: real media-node contract / system-suite entry point.
//!
//! This test drives the *same* [`run_media_port_contract`] suite that the
//! deterministic fake media node runs in CI, but against a live
//! [`SchedulerMediaPort`] backed by the real `cheetah.media.v1` gRPC control
//! client. It is `#[ignore]`d by default so it never runs in ordinary CI and
//! never reaches out to the network; it is only executed when an operator
//! provisions a real media node and points the suite at it via the
//! `CHEETAH_MEDIA_SYSTEM_ENDPOINT` environment variable, e.g.
//!
//! ```text
//! CHEETAH_MEDIA_SYSTEM_ENDPOINT=http://127.0.0.1:9000 \
//!   cargo test -p cheetah-media-scheduler --test media_port_system_test -- --ignored
//! ```
//!
//! The control plane never opens media ports or touches RTP/PS payloads here;
//! it only exercises the typed reserve/execute/release/query contract.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::media_contract::run_media_port_contract;
use cheetah_media_client::{MediaClientConfig, MediaControlClient};
use cheetah_media_scheduler::config::{MediaRegistryConfig, SchedulerConfig};
use cheetah_media_scheduler::model::{
    MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus,
};
use cheetah_media_scheduler::registry::{InMemoryMediaNodeRegistry, MediaNodeRegistry};
use cheetah_media_scheduler::scheduler::{LeastLoadedScheduler, MediaScheduler};
use cheetah_media_scheduler::{MediaMetrics, SchedulerMediaPort};
use cheetah_signal_types::test_support::{FakeClock, FakeIdGenerator};
use cheetah_signal_types::{IdGenerator, NodeId};

const ENDPOINT_ENV: &str = "CHEETAH_MEDIA_SYSTEM_ENDPOINT";

fn system_node(node_id: NodeId, control_endpoint: String) -> MediaNode {
    MediaNode {
        node_id,
        instance_id: "system-node-1".to_string(),
        instance_epoch: 1,
        zone: "zone-a".to_string(),
        region: "region-1".to_string(),
        network_zones: vec!["zone-a".to_string()],
        labels: Default::default(),
        control_endpoint,
        media_addresses: vec!["127.0.0.1:9001".to_string()],
        capabilities: vec![MediaCapability {
            protocol: "gb28181".to_string(),
            operations: vec![
                "live".to_string(),
                "playback".to_string(),
                "talk".to_string(),
                "broadcast".to_string(),
            ],
            constraints: Default::default(),
            version: 1,
            runtime_state: "active".to_string(),
        }],
        capacity: MediaNodeCapacity {
            max_sessions: 16,
            max_bandwidth_mbps: 1000,
            max_cpu_percent: 80,
        },
        load: 0,
        session_count: 0,
        health: MediaNodeHealth::Healthy,
        draining: false,
        status: NodeStatus::Active,
        last_heartbeat_at: None,
        lease_until: None,
        generation: 1,
        contract_version: 1,
        revision: 1,
    }
}

/// Runs the shared media-port contract against a real, provisioned media node.
///
/// Skips (and is `#[ignore]`d) unless `CHEETAH_MEDIA_SYSTEM_ENDPOINT` is set.
#[tokio::test]
#[ignore = "requires a provisioned real media node; set CHEETAH_MEDIA_SYSTEM_ENDPOINT"]
async fn real_media_node_satisfies_media_port_contract() {
    let Ok(endpoint) = std::env::var(ENDPOINT_ENV) else {
        tracing::info!(
            "skipping real media-node contract: set {ENDPOINT_ENV} to a media node control endpoint"
        );
        return;
    };

    let id_generator = Arc::new(FakeIdGenerator::new());
    let clock = FakeClock::new();

    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    let node = system_node(id_generator.generate_node_id(), endpoint.clone());
    registry
        .register(node, 60_000, &clock)
        .await
        .expect("registering the system media node must succeed");

    let scheduler: Arc<dyn MediaScheduler> = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));

    // Loopback/private endpoints and plain HTTP are permitted only for the
    // opt-in system suite; production wiring keeps these disabled.
    let client_config = MediaClientConfig {
        allow_insecure_http: endpoint.starts_with("http://"),
        allow_internal_endpoints: true,
        ..MediaClientConfig::default()
    };
    let client = MediaControlClient::new(client_config);

    let port = SchedulerMediaPort::new(scheduler, client, MediaMetrics::arc());

    run_media_port_contract(&port, id_generator.as_ref(), &clock).await;
}
