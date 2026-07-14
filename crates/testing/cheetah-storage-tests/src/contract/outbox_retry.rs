//! Outbox retry and failure contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_storage_api::Storage;

fn tenant_pending(
    pending: &[cheetah_domain::OutboxEntry],
    tenant_id: cheetah_signal_types::TenantId,
) -> Vec<cheetah_domain::OutboxEntry> {
    pending
        .iter()
        .filter(|e| e.event.tenant_id == tenant_id)
        .cloned()
        .collect()
}

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
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
    let tenant = tenant_pending(&pending, tenant_id);
    assert_eq!(tenant.len(), 1);
    assert_eq!(tenant[0].attempts, 0);

    let retry_at = fixtures
        .clock()
        .now_wall()
        .checked_add(cheetah_signal_types::DurationMs::from_millis(5));
    uow.outbox()
        .mark_failed(
            event.event_id,
            1,
            false,
            Some("transient".to_string()),
            retry_at,
        )
        .await?;
    uow.commit().await?;

    // With next_attempt_at in the future, the event should not be pending.
    fixtures.advance(cheetah_signal_types::DurationMs::from_millis(1));
    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    let tenant = tenant_pending(&pending, tenant_id);
    assert!(tenant.is_empty(), "future retry must not be pending");
    uow.commit().await?;

    // After the retry time passes, the event is pending again.
    fixtures.advance(cheetah_signal_types::DurationMs::from_millis(10));
    let mut uow = storage.begin().await?;
    let pending = uow
        .outbox()
        .pending(fixtures.clock().now_wall(), 10)
        .await?;
    let tenant = tenant_pending(&pending, tenant_id);
    assert_eq!(tenant.len(), 1);
    assert_eq!(tenant[0].attempts, 1);

    // Clean up so later tests do not see this event.
    uow.outbox().mark_published(event.event_id).await?;
    uow.commit().await?;

    Ok(())
}
