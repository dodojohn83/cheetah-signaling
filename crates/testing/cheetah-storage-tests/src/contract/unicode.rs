//! Unicode and special-character persistence tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{Channel, ChannelKind, ChannelStatus, Device, DeviceKind, Protocol};
use cheetah_signal_types::ProtocolIdentity;
use cheetah_storage_api::Storage;
use std::collections::BTreeMap;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let device_id = fixtures.device_id();
    let name = "摄像头 01 测-試_Émojis:🦌";
    let external_id = ProtocolIdentity::new("外部ID-Ω-Ü")
        .map_err(|e| cheetah_domain::DomainError::invalid_argument(e.to_string()))?;

    let (device, _event) = Device::new(
        fixtures.clock(),
        tenant_id,
        device_id,
        Protocol::Onvif,
        external_id,
        "authority",
        name,
        DeviceKind::Camera,
        vec![],
        BTreeMap::new(),
    )?;

    let mut uow = storage.begin().await?;
    uow.device_repository().save(&device).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .device_repository()
        .get_by_external_id(tenant_id, device.protocol(), device.external_id().clone())
        .await?
        .ok_or("unicode device not found")?;
    uow.commit().await?;

    assert_eq!(loaded.name(), name);
    assert_eq!(loaded.external_id().as_str(), "外部ID-Ω-Ü");

    let channel_name = "通道 1 — 測試 🎥";
    let channel_id = fixtures.id_generator().generate_channel_id();
    let (channel, _event) = Channel::new(
        fixtures.clock(),
        tenant_id,
        device_id,
        channel_id,
        ChannelKind::Video,
        channel_name,
        true,
        Some(ChannelStatus::Online),
        vec![],
        cheetah_domain::PtzCapabilities::new(false, false, false, false, false, false),
        BTreeMap::new(),
    )?;

    let mut uow = storage.begin().await?;
    uow.channel_repository().save(&channel).await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .channel_repository()
        .get(tenant_id, device_id, channel_id)
        .await?
        .ok_or("unicode channel not found")?;
    uow.commit().await?;

    assert_eq!(loaded.name(), channel_name);

    Ok(())
}
