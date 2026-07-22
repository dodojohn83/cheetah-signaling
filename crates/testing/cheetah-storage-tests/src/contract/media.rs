//! Media session and binding repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::DomainError;
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    session_crud(storage, fixtures).await?;
    binding_crud(storage, fixtures).await?;
    idempotency(storage, fixtures).await?;
    get_by_media_session(storage, fixtures).await?;
    revision_concurrency(storage, fixtures).await?;
    multi_bump_save_is_rejected(storage, fixtures).await?;
    Ok(())
}

async fn session_crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let session = fixtures.media_session(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .media_session_repository()
        .get(tenant_id, session.media_session_id())
        .await?
        .ok_or("media session not found after save")?;
    assert_eq!(loaded.device_id(), session.device_id());
    uow.commit().await?;

    Ok(())
}

async fn binding_crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let session = fixtures.media_session(tenant_id, device_id)?;
    let binding =
        fixtures.media_binding(tenant_id, session.media_session_id(), session.channel_id())?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.media_binding_repository().save(&binding).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .media_binding_repository()
        .get(tenant_id, binding.media_binding_id())
        .await?
        .ok_or("media binding not found after save")?;
    assert_eq!(loaded.media_session_id(), binding.media_session_id());
    uow.commit().await?;

    Ok(())
}

async fn idempotency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let session = fixtures.media_session(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .media_session_repository()
        .get_by_idempotency(session.idempotency_scope())
        .await?;
    assert!(
        loaded.is_some(),
        "media session must be found by idempotency scope"
    );
    uow.commit().await?;

    Ok(())
}

async fn get_by_media_session(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let session = fixtures.media_session(tenant_id, device_id)?;
    let binding =
        fixtures.media_binding(tenant_id, session.media_session_id(), session.channel_id())?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.media_binding_repository().save(&binding).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .media_binding_repository()
        .get_by_media_session(tenant_id, session.media_session_id())
        .await?;
    let loaded = loaded.ok_or("binding not found by media session id")?;
    assert_eq!(loaded.media_binding_id(), binding.media_binding_id());
    uow.commit().await?;

    Ok(())
}

async fn revision_concurrency(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let mut session = fixtures.media_session(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.commit().await?;

    session.allocating(fixtures.clock())?;

    let mut uow = storage.begin().await?;
    uow.media_session_repository().save(&session).await?;
    uow.commit().await?;

    let stale = session.clone();
    let mut uow = storage.begin().await?;
    let result = uow.media_session_repository().save(&stale).await;
    assert!(
        matches!(result, Err(DomainError::ConcurrentModification { .. })),
        "saving a stale media session must fail, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}

async fn multi_bump_save_is_rejected(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let device = fixtures.device(tenant_id, device_id)?;
    let mut session = fixtures.media_session(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.media_session_repository().save(&session).await?;
    uow.commit().await?;

    // Advance the session through two in-memory transitions without persisting
    // the first one. A single save at the end must fail because the stored
    // revision is two steps behind.
    session.allocating(fixtures.clock())?;
    session.inviting(fixtures.clock())?;

    let mut uow = storage.begin().await?;
    let result = uow.media_session_repository().save(&session).await;
    assert!(
        matches!(result, Err(DomainError::ConcurrentModification { .. })),
        "saving a session after multiple in-memory transitions without intermediate persist must fail, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}
