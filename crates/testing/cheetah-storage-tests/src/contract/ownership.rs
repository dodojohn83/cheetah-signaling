//! Unit-of-work ownership acquisition contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_signal_types::DurationMs;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    storage.migration().run().await?;

    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let this_node = fixtures.node_id();
    let this_instance = fixtures.node_instance_id();
    let other_node = fixtures.node_id();
    let other_instance = fixtures.node_instance_id();

    let node_repo = storage.node_repository();
    node_repo
        .register(fixtures.node(this_node, this_instance)?)
        .await?;
    node_repo
        .register(fixtures.node(other_node, other_instance)?)
        .await?;

    let now = fixtures.clock().now_wall();
    let lease_until = now
        .checked_add(DurationMs::from_millis(60_000))
        .ok_or("owner lease overflow")?;

    // 1. Fresh acquisition.
    let mut uow = storage.begin().await?;
    let (owner, previous) = uow
        .acquire_ownership(tenant_id, device_id, other_node, now, lease_until)
        .await?
        .ok_or("fresh acquire must succeed")?;
    assert_eq!(owner.owner_node_id, other_node);
    assert_eq!(owner.owner_epoch.0, 1);
    assert!(
        previous.is_none(),
        "fresh acquire must have no previous owner"
    );
    uow.commit().await?;

    // 2. Lost race against a live owner on another node.
    let mut uow = storage.begin().await?;
    let lost = uow
        .acquire_ownership(tenant_id, device_id, this_node, now, lease_until)
        .await?;
    assert!(lost.is_none(), "must lose race against live owner");
    uow.rollback().await?;

    // 3. Dead-node takeover after the owner node's lease expires.
    fixtures.advance(DurationMs::from_millis(120_000));
    let now = fixtures.clock().now_wall();
    let lease_until = now
        .checked_add(DurationMs::from_millis(60_000))
        .ok_or("owner lease overflow")?;

    // Re-register the candidate node so it is live at the new time.
    node_repo
        .register(fixtures.node(this_node, this_instance)?)
        .await?;

    let mut uow = storage.begin().await?;
    let (owner, previous) = uow
        .acquire_ownership(tenant_id, device_id, this_node, now, lease_until)
        .await?
        .ok_or("dead-node takeover must succeed")?;
    assert_eq!(owner.owner_node_id, this_node);
    assert_eq!(owner.owner_epoch.0, 2);
    assert_eq!(
        previous.map(|p| p.owner_node_id),
        Some(other_node),
        "takeover must report the previous owner"
    );
    uow.commit().await?;

    // 4. Re-acquisition by the same live node bumps the epoch.
    let mut uow = storage.begin().await?;
    let (owner, previous) = uow
        .acquire_ownership(tenant_id, device_id, this_node, now, lease_until)
        .await?
        .ok_or("same-node re-acquire must succeed")?;
    assert_eq!(owner.owner_node_id, this_node);
    assert_eq!(owner.owner_epoch.0, 3);
    assert_eq!(
        previous.map(|p| p.owner_node_id),
        Some(this_node),
        "re-acquire must report the previous owner"
    );
    uow.commit().await?;

    Ok(())
}
