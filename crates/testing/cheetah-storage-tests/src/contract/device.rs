//! Device repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::DomainError;
use cheetah_signal_types::TenantId;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    get_by_external_id(storage, fixtures).await?;
    revision_concurrency(storage, fixtures).await?;
    tenant_isolation(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .device_repository()
        .get(tenant_id, device_id)
        .await?
        .ok_or("device not found after save")?;
    assert_eq!(loaded.name(), device.name());
    assert_eq!(loaded.external_id().as_str(), device.external_id().as_str());
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let absent = uow
        .device_repository()
        .get(TenantId::default(), device_id)
        .await?;
    assert!(absent.is_none(), "wrong tenant must not see the device");
    uow.commit().await?;

    Ok(())
}

async fn get_by_external_id(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .device_repository()
        .get_by_external_id(tenant_id, device.protocol(), device.external_id().clone())
        .await?
        .ok_or("device not found by external id")?;
    assert_eq!(loaded.device_id(), device_id);
    uow.commit().await?;

    Ok(())
}

async fn revision_concurrency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut device = fixtures.device(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    device.update(
        fixtures.clock(),
        Some("renamed".to_string()),
        None,
        None,
        None,
        None,
        None,
        None,
    )?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let stale = device.clone();
    let mut uow = storage.begin().await?;
    let result = uow.device_repository().save(&stale).await;
    assert!(
        matches!(result, Err(DomainError::ConcurrentModification { .. })),
        "saving a stale revision must fail, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}

async fn tenant_isolation(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_a = fixtures.tenant_id();
    let tenant_b = fixtures.tenant_id();
    let device_a_id = fixtures.device_id();
    let device_b_id = fixtures.device_id();

    let device_a = fixtures.device(tenant_a, device_a_id)?;
    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_a).await?;
    uow.commit().await?;

    let device_b = fixtures.device(tenant_b, device_b_id)?;
    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_b).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let a = uow.device_repository().get(tenant_a, device_a_id).await?;
    let b = uow.device_repository().get(tenant_b, device_b_id).await?;
    let a_cross = uow.device_repository().get(tenant_a, device_b_id).await?;
    let b_cross = uow.device_repository().get(tenant_b, device_a_id).await?;
    uow.commit().await?;

    let a = a.ok_or("tenant_a device not found")?;
    let b = b.ok_or("tenant_b device not found")?;
    assert_eq!(a.tenant_id(), tenant_a);
    assert_eq!(b.tenant_id(), tenant_b);
    assert!(a_cross.is_none(), "tenant_a must not see tenant_b's device");
    assert!(b_cross.is_none(), "tenant_b must not see tenant_a's device");

    Ok(())
}
