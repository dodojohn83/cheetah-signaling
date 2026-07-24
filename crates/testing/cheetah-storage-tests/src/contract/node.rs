//! Cluster node repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{ClusterNode, NodeLoad};
use cheetah_signal_types::{DurationMs, PageRequest, UtcTimestamp};
use cheetah_storage_api::Storage;
use time::OffsetDateTime;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let node_id = fixtures.node_id();
    let instance_id = fixtures.node_instance_id();
    let node = fixtures.node(node_id, instance_id)?;

    let repo = storage.node_repository();
    repo.register(node.clone()).await?;

    let loaded = repo.get(node_id).await?;
    assert_eq!(loaded, Some(node.clone()));

    let lease_until = node
        .lease_until
        .checked_add(DurationMs::from_millis(60_000))
        .ok_or("lease overflow")?;
    let updated_at = node
        .updated_at
        .checked_add(DurationMs::from_millis(1_000))
        .ok_or("timestamp overflow")?;
    let heartbeat = repo
        .heartbeat(
            node_id,
            instance_id,
            lease_until,
            updated_at,
            NodeLoad { devices: 5 },
        )
        .await?;
    let heartbeat = heartbeat.ok_or("heartbeat should match registered instance")?;
    assert_eq!(heartbeat.load.devices, 5);

    // A heartbeat with a different instance should be rejected (fenced).
    let other_instance = fixtures.node_instance_id();
    let fenced = repo
        .heartbeat(
            node_id,
            other_instance,
            lease_until,
            updated_at,
            NodeLoad { devices: 0 },
        )
        .await?;
    assert!(fenced.is_none(), "other instance must be fenced");

    let drained = repo.mark_draining(node_id, instance_id, updated_at).await?;
    assert!(drained, "current instance should be able to mark draining");
    let after_drain = repo
        .get(node_id)
        .await?
        .ok_or("node should exist after drain")?;
    assert!(after_drain.draining);

    // list_alive should include the node while its lease is valid.
    let query_time = lease_until
        .checked_sub(DurationMs::from_millis(1))
        .ok_or("timestamp underflow")?;
    let page = PageRequest::new(10)?;
    let alive = repo.list_alive(query_time, page).await?;
    assert!(alive.items.iter().any(|n| n.node_id == node_id));

    // list_alive should exclude a node whose lease has expired.
    let expired = lease_until
        .checked_add(DurationMs::from_millis(1))
        .ok_or("timestamp overflow")?;
    let page = PageRequest::new(10)?;
    let alive = repo.list_alive(expired, page).await?;
    assert!(!alive.items.iter().any(|n| n.node_id == node_id));

    // Re-registration with a new instance overwrites the old one.
    let new_instance = fixtures.node_instance_id();
    let mut reregistered = ClusterNode::new(
        node_id,
        new_instance,
        "zone-a",
        "0.1.1",
        UtcTimestamp::from_offset(OffsetDateTime::UNIX_EPOCH),
    );
    reregistered.lease_until = node.lease_until;
    reregistered.updated_at = node.updated_at;
    repo.register(reregistered.clone()).await?;
    let loaded = repo
        .get(node_id)
        .await?
        .ok_or("node should exist after re-registration")?;
    assert_eq!(loaded.instance_id, new_instance);
    assert_eq!(loaded.version, "0.1.1");

    // Heartbeat with old instance should now be rejected.
    let fenced = repo
        .heartbeat(
            node_id,
            instance_id,
            lease_until,
            updated_at,
            NodeLoad { devices: 0 },
        )
        .await?;
    assert!(
        fenced.is_none(),
        "old instance must be fenced after re-registration"
    );

    // Mark draining with the old instance should also be rejected.
    let drained = repo.mark_draining(node_id, instance_id, updated_at).await?;
    assert!(
        !drained,
        "old instance must be fenced when marking draining"
    );

    Ok(())
}
