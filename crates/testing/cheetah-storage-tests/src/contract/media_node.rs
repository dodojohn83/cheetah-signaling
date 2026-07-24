//! Media node repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{
    DomainEvent, MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus,
};

use cheetah_signal_types::{
    DurationMs, Event, NodeId, PageRequest, Principal, PrincipalKind, RequestContext, ResourceId,
    ResourceKind, ResourceRef, TenantId, UtcTimestamp,
};
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

fn node_updated_event(fixtures: &Fixtures, node: &MediaNode) -> Event<DomainEvent> {
    let source = fixtures.node_id();
    let message_id = fixtures.id_generator().generate_message_id();
    let correlation_id = fixtures.id_generator().generate_correlation_id();
    let ctx = RequestContext {
        tenant_id: TenantId::default(),
        principal: Principal {
            id: source.to_string(),
            kind: PrincipalKind::Service,
            scopes: Vec::new(),
        },
        message_id,
        correlation_id,
        traceparent: None,
        tracestate: None,
        deadline: None,
        node_id: Some(source),
        source_ip: None,
    };
    let aggregate_ref = ResourceRef {
        tenant_id: TenantId::default(),
        kind: ResourceKind::MediaNode,
        id: ResourceId::MediaNode(node.node_id),
    };
    Event::new(
        fixtures.id_generator(),
        fixtures.clock(),
        &ctx,
        TenantId::default(),
        aggregate_ref,
        node.revision,
        DomainEvent::MediaNodeUpdated { node: node.clone() },
    )
}

async fn register_and_get(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);

    let event = node_updated_event(fixtures, &node);
    let persisted = repo.register(node, vec![event]).await?;
    assert_eq!(persisted.revision, 1);
    assert_media_node_outbox_event(storage, fixtures, node_id, 1).await?;

    let loaded = repo
        .get(node_id)
        .await?
        .ok_or("node not found after register")?;
    assert_eq!(loaded.node_id, node_id);
    assert_eq!(loaded.instance_id, "instance-1");
    assert_eq!(loaded.revision, 1);
    assert!(matches!(loaded.health, MediaNodeHealth::Healthy));

    let reregister_event = node_updated_event(fixtures, &loaded);
    let reregistered = repo.register(loaded, vec![reregister_event]).await?;
    assert_eq!(reregistered.revision, 2);
    assert_media_node_outbox_event(storage, fixtures, node_id, 2).await?;
    Ok(())
}

async fn heartbeat_and_lease(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    let event = node_updated_event(fixtures, &node);
    repo.register(node, vec![event]).await?;
    assert_media_node_outbox_event(storage, fixtures, node_id, 1).await?;

    fixtures.advance(DurationMs::from_millis(1_000));
    let now = fixtures.clock().now_wall();
    let lease = lease_at(now, 60_000)?;
    let mut heartbeat_node = sample_node(node_id);
    heartbeat_node.load = 20;
    heartbeat_node.session_count = 2;
    heartbeat_node.last_heartbeat_at = Some(now);
    heartbeat_node.lease_until = Some(lease);
    let heartbeat_event = node_updated_event(fixtures, &heartbeat_node);
    let updated = repo
        .heartbeat(
            node_id,
            "instance-1".to_string(),
            lease,
            now,
            20,
            2,
            vec![heartbeat_event],
        )
        .await?
        .ok_or("heartbeat failed")?;
    assert_eq!(updated.load, 20);
    assert_eq!(updated.session_count, 2);
    assert!(matches!(updated.health, MediaNodeHealth::Healthy));
    assert_media_node_outbox_event(storage, fixtures, node_id, 2).await?;

    let wrong = repo
        .heartbeat(
            node_id,
            "old-instance".to_string(),
            lease,
            now,
            50,
            0,
            vec![],
        )
        .await?;
    assert!(
        wrong.is_none(),
        "heartbeat from old instance must be fenced"
    );
    Ok(())
}

async fn list_alive(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    let event = node_updated_event(fixtures, &node);
    repo.register(node, vec![event]).await?;
    assert_media_node_outbox_event(storage, fixtures, node_id, 1).await?;

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
    let repo = storage.media_node_repository();
    let node_id = fixtures.node_id();
    let mut node = sample_node(node_id);
    let now = fixtures.clock().now_wall();
    node.last_heartbeat_at = Some(now);
    node.lease_until = Some(lease_at(now, 60_000)?);
    let event = node_updated_event(fixtures, &node);
    repo.register(node, vec![event]).await?;
    assert_media_node_outbox_event(storage, fixtures, node_id, 1).await?;

    let now = fixtures.clock().now_wall();
    let mut drain_node = sample_node(node_id);
    drain_node.draining = true;
    drain_node.status = NodeStatus::Draining;
    drain_node.last_heartbeat_at = Some(now);
    drain_node.lease_until = Some(lease_at(now, 60_000)?);
    let drain_event = node_updated_event(fixtures, &drain_node);
    let drained = repo
        .set_draining(
            node_id,
            "instance-1".to_string(),
            true,
            now,
            vec![drain_event],
        )
        .await?
        .ok_or("drain update failed")?;
    assert!(drained.draining);
    assert!(matches!(drained.status, NodeStatus::Draining));
    assert_eq!(drained.revision, 2, "drained revision");
    assert_media_node_outbox_event(storage, fixtures, node_id, 2).await?;

    let protection = lease_at(now, 60_000)?;
    let mut deregister_node = sample_node(node_id);
    deregister_node.status = NodeStatus::Left;
    deregister_node.lease_until = Some(protection);
    let deregister_event = node_updated_event(fixtures, &deregister_node);
    let deregistered = repo
        .deregister(
            node_id,
            "instance-1".to_string(),
            now,
            Some(protection),
            vec![deregister_event],
        )
        .await?
        .ok_or("deregister failed")?;
    assert!(matches!(deregistered.status, NodeStatus::Left));
    assert_eq!(deregistered.revision, 3, "deregistered revision");
    assert_media_node_outbox_event(storage, fixtures, node_id, 3).await?;

    // A deregistered node remains visible while its protection lease is valid.
    let alive = repo
        .list_alive(fixtures.clock().now_wall(), PageRequest::default())
        .await?;
    assert!(
        alive.items.iter().any(|n| n.node_id == node_id),
        "left node must remain in alive list during protection window"
    );

    // After the protection lease expires it is no longer returned.
    fixtures.advance(DurationMs::from_millis(60_001));
    let alive = repo
        .list_alive(fixtures.clock().now_wall(), PageRequest::default())
        .await?;
    assert!(
        !alive.items.iter().any(|n| n.node_id == node_id),
        "left node must not be alive after protection lease expires"
    );
    Ok(())
}

async fn assert_media_node_outbox_event(
    storage: &dyn Storage,
    fixtures: &Fixtures,
    node_id: NodeId,
    expected_revision: u64,
) -> TestResult<()> {
    let mut uow = storage.begin().await?;
    let entries = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 100)
        .await?;
    let node_events: Vec<_> = entries
        .into_iter()
        .filter(|entry| matches!(&entry.event.aggregate_ref.id, ResourceId::MediaNode(id) if *id == node_id))
        .collect();
    let entry = node_events
        .last()
        .ok_or("expected a media node outbox event")?;
    assert_eq!(entry.event.aggregate_sequence, expected_revision);
    if let DomainEvent::MediaNodeUpdated { node } = &entry.event.payload {
        assert_eq!(node.node_id, node_id);
        assert_eq!(node.revision, expected_revision);
    } else {
        return Err("expected MediaNodeUpdated outbox event".into());
    }
    // Mark the event published so later outbox contract tests do not count it.
    uow.outbox().mark_published(entry.event.event_id).await?;
    uow.commit().await?;

    // Verify the event no longer appears as pending.
    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 100)
        .await?;
    let still_pending = pending
        .into_iter()
        .any(|e| e.event.event_id == entry.event.event_id);
    assert!(!still_pending, "event should be marked published");
    uow.rollback().await?;
    Ok(())
}
