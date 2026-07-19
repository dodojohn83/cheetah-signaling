//! Operation repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::DomainError;
use cheetah_signal_types::TenantId;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    idempotency(storage, fixtures).await?;
    revision_concurrency(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let operation = fixtures.operation(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.operation_repository().save(&operation).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .operation_repository()
        .get(tenant_id, operation.operation_id())
        .await?
        .ok_or("operation not found after save")?;
    assert_eq!(loaded.device_id(), operation.device_id());
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let absent = uow
        .operation_repository()
        .get(TenantId::default(), operation.operation_id())
        .await?;
    assert!(absent.is_none(), "wrong tenant must not see the operation");
    uow.commit().await?;

    Ok(())
}

async fn idempotency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let operation = fixtures.operation(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.operation_repository().save(&operation).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .operation_repository()
        .get_by_idempotency(operation.command().idempotency_scope())
        .await?;
    assert!(
        loaded.is_some(),
        "operation must be found by idempotency scope"
    );
    uow.commit().await?;

    Ok(())
}

async fn revision_concurrency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let mut operation = fixtures.operation(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.operation_repository().save(&operation).await?;
    uow.commit().await?;

    operation.start(fixtures.clock())?;

    let mut uow = storage.begin().await?;
    uow.operation_repository().save(&operation).await?;
    uow.commit().await?;

    let stale = operation.clone();
    let mut uow = storage.begin().await?;
    let result = uow.operation_repository().save(&stale).await;
    assert!(
        matches!(
            result,
            Err(DomainError::ConcurrentModification {
                expected: 0,
                found: 1,
            })
        ),
        "saving a stale operation must report expected=0 and found=1, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}
