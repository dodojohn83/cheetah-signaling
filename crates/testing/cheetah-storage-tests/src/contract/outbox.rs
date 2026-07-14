//! Outbox repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    append_and_pending(storage, fixtures).await?;
    aggregate_and_outbox_same_transaction(storage, fixtures).await?;
    Ok(())
}

async fn append_and_pending(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let event = fixtures.outbox_event(tenant_id);

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.outbox().append(event.clone()).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    uow.commit().await?;

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event.event_id, event.event_id);
    assert!(!pending[0].published);

    let mut uow = storage.begin().await?;
    uow.outbox().mark_published(event.event_id).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    uow.commit().await?;

    assert!(pending.is_empty(), "marked events must not be pending");

    Ok(())
}

async fn aggregate_and_outbox_same_transaction(
    storage: &dyn Storage,
    fixtures: &Fixtures,
) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let event = fixtures.outbox_event(tenant_id);

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.outbox().append(event.clone()).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .device_repository()
        .get(tenant_id, device_id)
        .await?
        .ok_or("device not committed")?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    uow.commit().await?;

    assert_eq!(loaded.device_id(), device_id);
    let mut event_count = pending
        .iter()
        .filter(|e| e.event.tenant_id == tenant_id)
        .count();
    assert_eq!(event_count, 1);

    let mut uow = storage.begin().await?;
    uow.outbox().mark_published(event.event_id).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    uow.commit().await?;
    event_count = pending
        .iter()
        .filter(|e| e.event.tenant_id == tenant_id)
        .count();
    assert_eq!(event_count, 0, "test outbox event must be cleaned up");

    Ok(())
}
