//! Webhook repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::DomainError;
use cheetah_signal_types::DurationMs;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    tenant_isolation(storage, fixtures).await?;
    concurrent_save(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let config = fixtures.webhook_config(tenant_id)?;
    let webhook_id = config.webhook_id();

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&config).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .webhook_config_repository()
        .get(tenant_id, webhook_id)
        .await?
        .ok_or("webhook config not found after save")?;
    assert_eq!(loaded.url(), config.url());
    assert_eq!(loaded.revision().0, 0);
    uow.commit().await?;

    let mut updated = loaded.clone();
    updated.update(
        fixtures.clock(),
        Some("https://example.com/updated".to_string()),
        None,
        None,
        None,
    )?;
    assert_eq!(updated.revision().0, 1);

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&updated).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .webhook_config_repository()
        .get(tenant_id, webhook_id)
        .await?
        .ok_or("webhook config not found after update")?;
    assert_eq!(loaded.url(), "https://example.com/updated");
    assert_eq!(loaded.revision().0, 1);
    uow.commit().await?;

    Ok(())
}

async fn tenant_isolation(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_a = fixtures.tenant_id();
    let tenant_b = fixtures.tenant_id();
    let config = fixtures.webhook_config(tenant_a)?;
    let webhook_id = config.webhook_id();

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&config).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let foreign = uow
        .webhook_config_repository()
        .get(tenant_b, webhook_id)
        .await?;
    assert!(
        foreign.is_none(),
        "foreign tenant must not see webhook config"
    );
    uow.commit().await?;

    Ok(())
}

async fn concurrent_save(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let config = fixtures.webhook_config(tenant_id)?;
    let webhook_id = config.webhook_id();

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&config).await?;
    uow.commit().await?;

    // Keep a stale snapshot (revision 0) while the database row is moved to revision 1.
    let stale = config.clone();

    fixtures.advance(DurationMs::from_millis(1));

    let mut uow = storage.begin().await?;
    let mut updated = uow
        .webhook_config_repository()
        .get(tenant_id, webhook_id)
        .await?
        .ok_or("config not found")?;
    updated.update(
        fixtures.clock(),
        Some("https://example.com/updated".to_string()),
        None,
        None,
        None,
    )?;
    uow.webhook_config_repository().save(&updated).await?;
    uow.commit().await?;

    // Attempting to save the stale snapshot must fail with a concurrent modification error.
    let mut uow = storage.begin().await?;
    let result = uow.webhook_config_repository().save(&stale).await;
    assert!(
        matches!(result, Err(DomainError::ConcurrentModification { .. })),
        "saving stale webhook config must fail, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}
