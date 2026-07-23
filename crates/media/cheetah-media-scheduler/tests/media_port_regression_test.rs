//! Regression tests for `SchedulerMediaPort` command validation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::{CommandPayload, MediaClient, MediaNodeCommand, MediaPort};
use cheetah_media_client::MediaControlClient;
use cheetah_media_scheduler::config::{MediaRegistryConfig, SchedulerConfig};
use cheetah_media_scheduler::model::{
    MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus,
};
use cheetah_media_scheduler::registry::{InMemoryMediaNodeRegistry, MediaNodeRegistry};
use cheetah_media_scheduler::scheduler::LeastLoadedScheduler;
use cheetah_media_scheduler::{MediaMetrics, SchedulerMediaPort};
use cheetah_signal_types::test_support::FakeClock;
use cheetah_signal_types::{
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OperationId, OwnerEpoch,
    TenantId,
};
use std::str::FromStr;
use std::sync::Arc;

fn tenant_id() -> TenantId {
    TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap()
}

fn node_id() -> NodeId {
    NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap()
}

fn session_id() -> MediaSessionId {
    MediaSessionId::from_str("44444444-4444-4444-4444-444444444444").unwrap()
}

fn binding_id() -> MediaBindingId {
    MediaBindingId::from_str("33333333-3333-3333-3333-333333333333").unwrap()
}

fn default_node() -> MediaNode {
    MediaNode {
        node_id: node_id(),
        instance_id: "instance-1".to_string(),
        instance_epoch: 1,
        zone: "zone-a".to_string(),
        region: "region-1".to_string(),
        network_zones: vec!["zone-a".to_string()],
        labels: Default::default(),
        control_endpoint: "http://127.0.0.1:9000".to_string(),
        media_addresses: vec!["127.0.0.1:9001".to_string()],
        capabilities: vec![MediaCapability {
            protocol: "gb28181".to_string(),
            operations: vec!["live".to_string(), "playback".to_string()],
            constraints: Default::default(),
            version: 1,
            runtime_state: "healthy".to_string(),
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

fn stop_command(owner_epoch: OwnerEpoch) -> MediaNodeCommand {
    MediaNodeCommand {
        request_id: "req-1".to_string(),
        tenant_id: tenant_id(),
        media_session_id: session_id(),
        media_binding_id: binding_id(),
        media_node_id: node_id(),
        media_node_instance_epoch: MediaNodeInstanceEpoch(1),
        operation_id: OperationId::from_str("55555555-5555-5555-5555-555555555555").unwrap(),
        owner_epoch,
        source_node_id: node_id(),
        deadline: None,
        idempotency_key: "idem-1".to_string(),
        contract_version: 1,
        payload: CommandPayload::StopMediaSession {
            media_session_id: session_id(),
        },
    }
}

#[tokio::test]
async fn scheduler_port_rejects_orphan_stop_with_zero_owner_epoch() {
    let clock = Arc::new(FakeClock::new());
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    registry
        .register(default_node(), 60_000, clock.as_ref())
        .await
        .unwrap();
    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));
    let client: Arc<dyn MediaClient> = Arc::new(MediaControlClient::new(Default::default()));
    let port = SchedulerMediaPort::new(scheduler, client, Arc::new(MediaMetrics::new()));

    let err = port
        .execute(stop_command(OwnerEpoch(0)), clock.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("owner_epoch is required"));
}

#[tokio::test]
async fn scheduler_port_accepts_orphan_stop_with_non_zero_owner_epoch() {
    let clock = Arc::new(FakeClock::new());
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    registry
        .register(default_node(), 60_000, clock.as_ref())
        .await
        .unwrap();
    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));
    let client: Arc<dyn MediaClient> = Arc::new(MediaControlClient::new(Default::default()));
    let port = SchedulerMediaPort::new(scheduler, client, Arc::new(MediaMetrics::new()));

    // With a non-zero owner epoch the command passes local validation and is
    // handed to the control client. The endpoint is unreachable, so the client
    // returns a connection error rather than an owner-epoch rejection.
    let err = port
        .execute(stop_command(OwnerEpoch(1)), clock.as_ref())
        .await
        .unwrap_err();
    assert!(!err.to_string().contains("owner_epoch is required"));
}
