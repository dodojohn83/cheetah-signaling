//! Paginated list repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_signal_types::{PageRequest, TenantId};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    device_list(storage, fixtures).await?;
    channel_list(storage, fixtures).await?;
    operation_list(storage, fixtures).await?;
    media_session_list(storage, fixtures).await?;
    Ok(())
}

async fn device_list(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let mut uow = storage.begin().await?;

    let mut devices = Vec::new();
    for i in 0..5 {
        let device_id = fixtures.device_id();
        let mut device = fixtures.device(tenant_id, device_id)?;
        if i == 1 {
            device.update(
                fixtures.clock(),
                Some("alpha device".to_string()),
                None,
                None,
                None,
                None,
                None,
                None,
            )?;
        }
        uow.device_repository().save(&device).await?;
        devices.push(device);
    }
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let page = uow
        .device_repository()
        .list(tenant_id, None, None, None, None, PageRequest::new(2)?)
        .await?;
    uow.commit().await?;
    assert_eq!(page.items.len(), 2, "page size must be respected");
    assert!(page.next_cursor.is_some(), "next cursor must be present");

    let mut uow = storage.begin().await?;
    let page2 = uow
        .device_repository()
        .list(
            tenant_id,
            None,
            None,
            None,
            None,
            PageRequest::new(2)?.with_cursor(
                page.next_cursor
                    .ok_or("missing next cursor on first page")?,
            ),
        )
        .await?;
    uow.commit().await?;
    assert_eq!(page2.items.len(), 2);
    assert!(page2.next_cursor.is_some());

    let mut uow = storage.begin().await?;
    let prefix_page = uow
        .device_repository()
        .list(
            tenant_id,
            None,
            None,
            Some("alpha".to_string()),
            None,
            PageRequest::new(10)?,
        )
        .await?;
    uow.commit().await?;
    assert_eq!(prefix_page.items.len(), 1, "name prefix filter must match");

    let mut uow = storage.begin().await?;
    let other_tenant = TenantId::default();
    let empty = uow
        .device_repository()
        .list(other_tenant, None, None, None, None, PageRequest::new(10)?)
        .await?;
    uow.commit().await?;
    assert!(empty.items.is_empty(), "tenant isolation must hold");

    Ok(())
}

async fn channel_list(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();

    let mut uow = storage.begin().await?;
    let device = fixtures.device(tenant_id, device_id)?;
    uow.device_repository().save(&device).await?;
    for _ in 0..3 {
        let channel = fixtures.channel(tenant_id, device_id)?;
        uow.channel_repository().save(&channel).await?;
    }
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let page = uow
        .channel_repository()
        .list(tenant_id, device_id, None, None, None, PageRequest::new(2)?)
        .await?;
    uow.commit().await?;
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_some());

    Ok(())
}

async fn operation_list(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();

    let mut uow = storage.begin().await?;
    let device = fixtures.device(tenant_id, device_id)?;
    uow.device_repository().save(&device).await?;
    for _ in 0..3 {
        let operation = fixtures.operation(tenant_id, device_id)?;
        uow.operation_repository().save(&operation).await?;
    }
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let page = uow
        .operation_repository()
        .list(tenant_id, Some(device_id), None, None, PageRequest::new(2)?)
        .await?;
    uow.commit().await?;
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_some());

    Ok(())
}

async fn media_session_list(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();

    let mut uow = storage.begin().await?;
    let device = fixtures.device(tenant_id, device_id)?;
    uow.device_repository().save(&device).await?;
    for _ in 0..3 {
        let session = fixtures.media_session(tenant_id, device_id)?;
        uow.media_session_repository().save(&session).await?;
    }
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let page = uow
        .media_session_repository()
        .list(
            tenant_id,
            Some(device_id),
            None,
            None,
            None,
            PageRequest::new(2)?,
        )
        .await?;
    uow.commit().await?;
    assert_eq!(page.items.len(), 2);
    assert!(page.next_cursor.is_some());

    Ok(())
}
