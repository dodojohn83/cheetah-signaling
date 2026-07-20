//! Scheduler integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::MediaRequirements;
use cheetah_media_scheduler::config::{MediaRegistryConfig, SchedulerConfig};
use cheetah_media_scheduler::model::{
    MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus,
};
use cheetah_media_scheduler::registry::{InMemoryMediaNodeRegistry, MediaNodeRegistry};
use cheetah_media_scheduler::scheduler::{LeastLoadedScheduler, MediaScheduler};
use cheetah_signal_types::{
    Clock, DurationMs, MediaBindingId, MediaSessionId, NodeId, TenantId, UtcTimestamp,
};
use std::str::FromStr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use time::OffsetDateTime;

struct ManualClock {
    wall: Mutex<OffsetDateTime>,
    mono: AtomicI64,
}

impl ManualClock {
    fn new(wall: OffsetDateTime) -> Self {
        Self {
            wall: Mutex::new(wall),
            mono: AtomicI64::new(0),
        }
    }

    fn advance_wall(&self, millis: i64) {
        let mut wall = self.wall.lock().unwrap();
        *wall = wall
            .checked_add(time::Duration::milliseconds(millis))
            .unwrap();
    }
}

impl Clock for ManualClock {
    fn now_wall(&self) -> UtcTimestamp {
        UtcTimestamp::from_offset(*self.wall.lock().unwrap())
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::from_millis(self.mono.fetch_add(1, Ordering::SeqCst))
    }
}

fn tenant_id() -> TenantId {
    TenantId::from_str("11111111-1111-1111-1111-111111111111").unwrap()
}

fn node_id() -> NodeId {
    NodeId::from_str("22222222-2222-2222-2222-222222222222").unwrap()
}

fn node_b_id() -> NodeId {
    NodeId::from_str("22222222-2222-2222-2222-222222222223").unwrap()
}

fn binding_id() -> MediaBindingId {
    MediaBindingId::from_str("33333333-3333-3333-3333-333333333333").unwrap()
}

fn session_id() -> MediaSessionId {
    MediaSessionId::from_str("44444444-4444-4444-4444-444444444444").unwrap()
}

fn node_with_contract_version(version: &str) -> MediaNode {
    let mut node = default_node();
    node.instance_id = format!("instance-{version}");
    node.node_id =
        NodeId::from_str(&format!("22222222-2222-2222-2222-22222222222{version}")).unwrap();
    for cap in &mut node.capabilities {
        cap.constraints
            .insert("contract_version".to_string(), version.to_string());
    }
    node
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
            operations: vec![
                "live".to_string(),
                "playback".to_string(),
                "talk".to_string(),
            ],
            constraints: Default::default(),
            version: 1,
            runtime_state: "active".to_string(),
        }],
        capacity: MediaNodeCapacity {
            max_sessions: 4,
            max_bandwidth_mbps: 1000,
            max_cpu_percent: 80,
        },
        load: 10,
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

fn requirements(operation: &str, session: Option<MediaSessionId>) -> MediaRequirements {
    MediaRequirements {
        protocol: "gb28181".to_string(),
        operation: operation.to_string(),
        transport: None,
        encapsulation: None,
        codecs: vec![],
        session_type: operation.to_string(),
        zone: None,
        network_zone: None,
        tenant_constraints: Default::default(),
        required_constraints: Default::default(),
        media_session_id: session.map(|s| s.to_string()),
    }
}

#[tokio::test]
async fn register_node_and_reserve_binding() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    let node = default_node();
    registry
        .register(node, 60_000, clock.as_ref())
        .await
        .unwrap();

    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));
    let chosen = scheduler
        .schedule(
            tenant_id(),
            &requirements("live", None),
            &[],
            clock.as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(chosen.node_id, node_id());

    let reserved = scheduler
        .reserve(
            chosen.node_id,
            tenant_id(),
            binding_id(),
            &requirements("live", Some(session_id())),
            clock.as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(reserved.node_id, node_id());
    assert_eq!(reserved.session_count, 1);

    scheduler
        .release(tenant_id(), binding_id(), clock.as_ref())
        .await
        .unwrap();
}

#[tokio::test]
async fn wrong_protocol_returns_no_node() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
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

    let mut req = requirements("live", None);
    req.protocol = "onvif".to_string();
    let err = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no node satisfies"));
}

#[tokio::test]
async fn draining_node_not_chosen_for_new_sessions() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    registry
        .register(default_node(), 60_000, clock.as_ref())
        .await
        .unwrap();

    registry
        .drain(node_id(), true, clock.as_ref())
        .await
        .unwrap();

    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));
    let err = scheduler
        .schedule(
            tenant_id(),
            &requirements("live", None),
            &[],
            clock.as_ref(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no node satisfies"));
}

#[tokio::test]
async fn lease_expired_node_is_not_scheduled() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let config = MediaRegistryConfig {
        heartbeat_timeout_ms: 1_000,
        ..Default::default()
    };
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(config));
    registry
        .register(default_node(), 60_000, clock.as_ref())
        .await
        .unwrap();

    clock.advance_wall(2_000);

    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry,
        SchedulerConfig::default(),
    ));
    let err = scheduler
        .schedule(
            tenant_id(),
            &requirements("live", None),
            &[],
            clock.as_ref(),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no registered media nodes"));
}

#[tokio::test]
async fn same_session_prefers_original_node() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
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
    let session = session_id();
    let req = requirements("live", Some(session));
    let chosen = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap();
    scheduler
        .reserve(
            chosen.node_id,
            tenant_id(),
            binding_id(),
            &req,
            clock.as_ref(),
        )
        .await
        .unwrap();

    let other_binding = MediaBindingId::from_str("55555555-5555-5555-5555-555555555555").unwrap();
    let chosen_again = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(chosen.node_id, chosen_again.node_id);

    scheduler
        .reserve(
            chosen_again.node_id,
            tenant_id(),
            other_binding,
            &req,
            clock.as_ref(),
        )
        .await
        .unwrap();

    scheduler
        .release(tenant_id(), binding_id(), clock.as_ref())
        .await
        .unwrap();
    scheduler
        .release(tenant_id(), other_binding, clock.as_ref())
        .await
        .unwrap();
}

#[tokio::test]
async fn capacity_exhausted_falls_back_or_fails() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
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
    let session = session_id();
    let req = requirements("live", Some(session));
    let node = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap();

    for i in 0..4 {
        let binding =
            MediaBindingId::from_str(&format!("55555555-5555-5555-5555-{i:012}")).unwrap();
        scheduler
            .reserve(node.node_id, tenant_id(), binding, &req, clock.as_ref())
            .await
            .unwrap();
    }

    let extra = MediaBindingId::from_str("66666666-6666-6666-6666-666666666666").unwrap();
    let err = scheduler
        .reserve(node.node_id, tenant_id(), extra, &req, clock.as_ref())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        cheetah_media_scheduler::error::SchedulerError::CapacityExhausted(_)
    ));
}

#[tokio::test]
async fn scale_out_avoids_draining_node_and_breaks_affinity() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));

    let node_a = default_node();
    registry
        .register(node_a.clone(), 60_000, clock.as_ref())
        .await
        .unwrap();

    // Register node B but immediately mark it as Draining so the first session
    // deterministically lands on node A.
    let mut node_b = default_node();
    node_b.node_id = node_b_id();
    node_b.instance_id = "instance-2".to_string();
    node_b.load = 5;
    registry
        .register(node_b.clone(), 60_000, clock.as_ref())
        .await
        .unwrap();
    registry
        .drain(node_b.node_id, true, clock.as_ref())
        .await
        .unwrap();

    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry.clone(),
        SchedulerConfig::default(),
    ));

    // Initial session is scheduled on node A and reserved.
    let session = session_id();
    let req = requirements("live", Some(session));
    let chosen = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(chosen.node_id, node_a.node_id);

    scheduler
        .reserve(
            chosen.node_id,
            tenant_id(),
            binding_id(),
            &req,
            clock.as_ref(),
        )
        .await
        .unwrap();

    // Scale out: re-enable node B, then drain node A. New work must go to node B
    // and affinity on node A is broken because it is no longer eligible.
    registry
        .drain(node_b.node_id, false, clock.as_ref())
        .await
        .unwrap();
    registry
        .drain(node_a.node_id, true, clock.as_ref())
        .await
        .unwrap();

    let migrated = scheduler
        .schedule(tenant_id(), &req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(migrated.node_id, node_b.node_id);

    // A completely new session is also scheduled on node B.
    let new_session = MediaSessionId::from_str("44444444-4444-4444-4444-444444444445").unwrap();
    let new_req = requirements("live", Some(new_session));
    let new_chosen = scheduler
        .schedule(tenant_id(), &new_req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(new_chosen.node_id, node_b.node_id);

    // Node A remains listed because it is only draining (not Left or expired)
    // and the frozen clock never exceeds its 60s lease.
    let active_nodes = scheduler.list_nodes(clock.as_ref()).await;
    assert!(active_nodes.contains(&node_a.node_id));
    assert!(active_nodes.contains(&node_b.node_id));
}

#[tokio::test]
async fn rolling_upgrade_routes_new_sessions_to_newer_contract_version() {
    let clock = Arc::new(ManualClock::new(OffsetDateTime::UNIX_EPOCH));
    let registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));

    let node_a = node_with_contract_version("1");
    registry
        .register(node_a.clone(), 60_000, clock.as_ref())
        .await
        .unwrap();

    let scheduler = Arc::new(LeastLoadedScheduler::new(
        registry.clone(),
        SchedulerConfig::default(),
    ));

    // Existing session lands on node A (contract v1) without a contract requirement.
    let session = session_id();
    let mut old_req = requirements("live", Some(session));
    let chosen = scheduler
        .schedule(tenant_id(), &old_req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(chosen.node_id, node_a.node_id);

    scheduler
        .reserve(
            chosen.node_id,
            tenant_id(),
            binding_id(),
            &old_req,
            clock.as_ref(),
        )
        .await
        .unwrap();

    // A new node joins with contract v2 (rolling upgrade scale-out).
    let node_b = node_with_contract_version("2");
    registry
        .register(node_b.clone(), 60_000, clock.as_ref())
        .await
        .unwrap();

    // New sessions requiring contract v2 must be scheduled on node B.
    let new_session = MediaSessionId::from_str("44444444-4444-4444-4444-444444444445").unwrap();
    let mut new_req = requirements("live", Some(new_session));
    new_req
        .required_constraints
        .insert("contract_version".to_string(), "2".to_string());
    let v2_chosen = scheduler
        .schedule(tenant_id(), &new_req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(v2_chosen.node_id, node_b.node_id);

    // Re-scheduling the old session with a v2 requirement breaks affinity and migrates to B.
    old_req
        .required_constraints
        .insert("contract_version".to_string(), "2".to_string());
    let migrated = scheduler
        .schedule(tenant_id(), &old_req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(migrated.node_id, node_b.node_id);

    // Node A is still available for sessions that only need contract v1.
    let mut v1_req = requirements("live", None);
    v1_req
        .required_constraints
        .insert("contract_version".to_string(), "1".to_string());
    let v1_chosen = scheduler
        .schedule(tenant_id(), &v1_req, &[], clock.as_ref())
        .await
        .unwrap();
    assert_eq!(v1_chosen.node_id, node_a.node_id);
}
