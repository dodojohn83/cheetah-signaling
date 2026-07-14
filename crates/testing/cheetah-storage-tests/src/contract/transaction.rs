//! Transaction rollback contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let event = fixtures.outbox_event(tenant_id);

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.outbox().append(event.clone()).await?;
    uow.rollback().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow.device_repository().get(tenant_id, device_id).await?;
    let pending = uow.outbox().pending(10).await?;
    uow.commit().await?;

    assert!(loaded.is_none(), "rolled-back device must not be persisted");
    assert!(
        pending.iter().all(|e| e.event.event_id != event.event_id),
        "rolled-back outbox event must not be persisted"
    );

    Ok(())
}
