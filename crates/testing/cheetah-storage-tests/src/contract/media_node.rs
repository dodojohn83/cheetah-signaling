//! Media node repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
use cheetah_signal_types::{DurationMs, NodeId, PageRequest, UtcTimestamp};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    register_and_get(storage, fixtures).await?;
    heartbeat_and_lease(storage, fixtures).await?;
    list_alive(storage, fixtures).await?;
    drain_and_deregister(storage, fixtures).await?;
    Ok(())
}

fn lease_at(now: UtcTimestamp, ms: i64) -> TestResult<UtcTimestamp> {
    now.checked_add(DurationMs::from_millis(ms))
        .ok_or("test lease overflow".into())
}

fn sample_node(node_id: NodeId) -> MediaNode {
    MediaNode {
        node_id,
        instance_id: "instance-1".to_string(),
        instance_epoch: 1,
        zone: "zone-a".to_string(),
        region: "region-1".to_string(),
        network_zones: vec!["net-a".to_string()],
        labels: Default::default(),
        control_endpoint: "https://1.2.3.4:5000".to_string(),
        media_addresses: vec!["rtp://1.2.3.4:10000".to_string()],
        capabilities: vec![MediaCapability {
            protocol: "gb28181".to_string(),
            operations: vec!["live".to_string()],
            constraints: Default::default(),
            version: 1,
            runtime_state: "healthy".to_string(),
        }],
        capacity: MediaNodeCapacity {
            max_sessions: 10,
            max_bandwidth_mbps: 100,
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
        revision: 0,
    }
}

async fn register_and_get(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let mut repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);

    let persisted = repo.register(node).await?;
    assert_eq!(persisted.revision, 1);

    let loaded = repo
        .get(node_id)
        .await?
        .ok_or("node not found after register")?;
    assert_eq!(loaded.node_id, node_id);
    assert_eq!(loaded.instance_id, "instance-1");
    assert_eq!(loaded.revision, 1);
    assert!(matches!(loaded.health, MediaNodeHealth::Healthy));

    let reregistered = repo.register(loaded).await?;
    assert_eq!(reregistered.revision, 2);
    Ok(())
}

async fn heartbeat_and_lease(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let mut repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    repo.register(node).await?;

    fixtures.advance(DurationMs::from_millis(1_000));
    let now = fixtures.clock().now_wall();
    let lease = lease_at(now, 60_000)?;
    let updated = repo
        .heartbeat(node_id, "instance-1".to_string(), lease, now, 20, 2)
        .await?
        .ok_or("heartbeat failed")?;
    assert_eq!(updated.load, 20);
    assert_eq!(updated.session_count, 2);
    assert!(matches!(updated.health, MediaNodeHealth::Healthy));

    let wrong = repo
        .heartbeat(node_id, "old-instance".to_string(), lease, now, 50, 0)
        .await?;
    assert!(
        wrong.is_none(),
        "heartbeat from old instance must be fenced"
    );
    Ok(())
}

async fn list_alive(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let mut repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    repo.register(node).await?;

    let page = repo
        .list_alive(fixtures.clock().now_wall(), PageRequest::default())
        .await?;
    assert!(
        page.items.iter().any(|n| n.node_id == node_id),
        "node must be in alive list"
    );
    Ok(())
}

async fn drain_and_deregister(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let mut repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    repo.register(node).await?;

    let now = fixtures.clock().now_wall();
    let drained = repo
        .set_draining(node_id, "instance-1".to_string(), true, now)
        .await?
        .ok_or("drain update failed")?;
    assert!(drained.draining);
    assert!(matches!(drained.status, NodeStatus::Draining));

    let deregistered = repo
        .deregister(
            node_id,
            "instance-1".to_string(),
            now,
            Some(lease_at(now, 60_000)?),
        )
        .await?
        .ok_or("deregister failed")?;
    assert!(matches!(deregistered.status, NodeStatus::Left));

    let alive = repo
        .list_alive(fixtures.clock().now_wall(), PageRequest::default())
        .await?;
    assert!(
        !alive.items.iter().any(|n| n.node_id == node_id),
        "left node must not be alive"
    );
    Ok(())
}
