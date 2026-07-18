//! Channel repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{Channel, ChannelKind, ChannelStatus, PtzCapabilities};
use cheetah_storage_api::Storage;
use std::collections::BTreeMap;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    crud(storage, fixtures).await?;
    list_by_device(storage, fixtures).await?;
    remove(storage, fixtures).await?;
    remove_and_recreate(storage, fixtures).await?;
    tenant_isolation(storage, fixtures).await?;
    Ok(())
}

async fn crud(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut device = fixtures.device(tenant_id, device_id)?;
    device.mark_online(fixtures.clock(), None)?;

    let channel = fixtures.channel(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.channel_repository().save(&channel).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .channel_repository()
        .get(tenant_id, device_id, channel.channel_id())
        .await?
        .ok_or("channel not found after save")?;
    assert_eq!(loaded.name(), channel.name());
    assert_eq!(loaded.enabled(), channel.enabled());
    uow.commit().await?;

    Ok(())
}

async fn list_by_device(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut device = fixtures.device(tenant_id, device_id)?;
    device.mark_online(fixtures.clock(), None)?;

    let channel_a = fixtures.channel(tenant_id, device_id)?;
    let channel_b = fixtures.channel(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.channel_repository().save(&channel_a).await?;
    uow.channel_repository().save(&channel_b).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let channels = uow
        .channel_repository()
        .list_by_device(tenant_id, device_id)
        .await?;
    uow.commit().await?;

    let ids: Vec<_> = channels.iter().map(|c| c.channel_id()).collect();
    assert!(ids.contains(&channel_a.channel_id()));
    assert!(ids.contains(&channel_b.channel_id()));

    Ok(())
}

async fn remove(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut device = fixtures.device(tenant_id, device_id)?;
    device.mark_online(fixtures.clock(), None)?;
    let channel = fixtures.channel(tenant_id, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.channel_repository().save(&channel).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    uow.channel_repository()
        .remove(tenant_id, device_id, channel.channel_id())
        .await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .channel_repository()
        .get(tenant_id, device_id, channel.channel_id())
        .await?;
    assert!(loaded.is_none(), "removed channel must not be found");
    uow.commit().await?;

    Ok(())
}

async fn remove_and_recreate(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let mut device = fixtures.device(tenant_id, device_id)?;
    device.mark_online(fixtures.clock(), None)?;
    let channel = fixtures.channel(tenant_id, device_id)?;
    let channel_id = channel.channel_id();

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.channel_repository().save(&channel).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    uow.channel_repository()
        .remove(tenant_id, device_id, channel_id)
        .await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .channel_repository()
        .get(tenant_id, device_id, channel_id)
        .await?;
    assert!(loaded.is_none(), "removed channel must not be found");
    uow.commit().await?;

    // Re-introducing the same channel id after removal must succeed.
    let (recreate, _event) = Channel::new(
        fixtures.clock(),
        tenant_id,
        device_id,
        channel_id,
        ChannelKind::Video,
        "recreated stream",
        true,
        Some(ChannelStatus::Online),
        Vec::new(),
        PtzCapabilities::new(false, false, false, false, false, false),
        BTreeMap::new(),
    )?;

    let mut uow = storage.begin().await?;
    uow.channel_repository().save(&recreate).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .channel_repository()
        .get(tenant_id, device_id, channel_id)
        .await?
        .ok_or("recreated channel not found")?;
    assert_eq!(loaded.name(), "recreated stream");
    uow.commit().await?;

    Ok(())
}

async fn tenant_isolation(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_a = fixtures.tenant_id();
    let tenant_b = fixtures.tenant_id();
    let device_id = fixtures.device_id();

    let mut device_a = fixtures.device(tenant_a, device_id)?;
    device_a.mark_online(fixtures.clock(), None)?;
    let channel_a = fixtures.channel(tenant_a, device_id)?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device_a).await?;
    uow.channel_repository().save(&channel_a).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let foreign = uow
        .channel_repository()
        .get(tenant_b, device_id, channel_a.channel_id())
        .await?;
    assert!(
        foreign.is_none(),
        "foreign tenant channel must not be visible"
    );
    uow.commit().await?;

    Ok(())
}
