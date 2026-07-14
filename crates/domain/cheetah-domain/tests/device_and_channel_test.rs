//! Device and Channel aggregate lifecycle tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{
    Capability, CapabilityValue, Channel, ChannelKind, ChannelStatus, Device, DeviceKind,
    DeviceLifecycle, PtzCapabilities,
};
use cheetah_signal_types::{
    ChannelId, DeviceId, DurationMs, IdGenerator, ProtocolIdentity, TenantId,
};

fn setup() -> (
    InMemoryClock,
    InMemoryIdGenerator,
    TenantId,
    DeviceId,
    ChannelId,
) {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let channel_id = id_generator.generate_channel_id();
    (clock, id_generator, tenant_id, device_id, channel_id)
}

fn new_device(clock: &InMemoryClock, tenant_id: TenantId, device_id: DeviceId) -> Device {
    let (device, _) = Device::new(
        clock,
        tenant_id,
        device_id,
        cheetah_domain::Protocol::Gb28181,
        ProtocolIdentity::new("ext-1").unwrap(),
        "authority",
        "camera-01",
        DeviceKind::Camera,
        Vec::new(),
        BTreeMap::new(),
    )
    .expect("device creation should succeed");
    device
}

fn new_channel(
    clock: &InMemoryClock,
    tenant_id: TenantId,
    device_id: DeviceId,
    channel_id: ChannelId,
) -> Channel {
    let (channel, _) = Channel::new(
        clock,
        tenant_id,
        device_id,
        channel_id,
        ChannelKind::Video,
        "channel-01",
        true,
        Some(ChannelStatus::Online),
        Vec::new(),
        PtzCapabilities::new(false, false, false, false, false, false),
        BTreeMap::new(),
    )
    .expect("channel creation should succeed");
    channel
}

#[test]
fn device_starts_provisioning() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let device = new_device(&clock, tenant_id, device_id);
    assert_eq!(device.lifecycle(), DeviceLifecycle::Provisioning);
    assert_eq!(device.connectivity(), cheetah_domain::Connectivity::Unknown);
    assert_eq!(device.owner_epoch().0, 0);
}

#[test]
fn device_mark_online_activates_and_sets_connectivity() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    let event = device
        .mark_online(&clock, None)
        .expect("mark_online should succeed");
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::DeviceOnlineChanged { .. }
    ));
    assert_eq!(device.lifecycle(), DeviceLifecycle::Active);
    assert_eq!(device.connectivity(), cheetah_domain::Connectivity::Online);
}

#[test]
fn device_mark_offline_sets_offline_reason() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    device.mark_online(&clock, None).unwrap();
    let event = device
        .mark_offline(&clock, "network-lost".to_string())
        .unwrap();
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::DeviceOnlineChanged { .. }
    ));
    assert!(
        matches!(device.connectivity(), cheetah_domain::Connectivity::Offline { reason, .. } if reason == "network-lost")
    );
}

#[test]
fn device_suspend_and_activate_lifecycle() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    device.mark_online(&clock, None).unwrap();
    device.suspend(&clock).unwrap();
    assert_eq!(device.lifecycle(), DeviceLifecycle::Suspended);
    device.activate(&clock).unwrap();
    assert_eq!(device.lifecycle(), DeviceLifecycle::Active);
}

#[test]
fn device_retire_makes_device_immutable() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    device.retire(&clock).unwrap();
    assert_eq!(device.lifecycle(), DeviceLifecycle::Retired);
    let err = device.mark_online(&clock, None).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::InvalidTransition { .. }
    ));
    let err = device.replace_capabilities(&clock, Vec::new()).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::InvalidTransition { .. }
    ));
}

#[test]
fn device_replace_capabilities_and_metadata() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    let capability = Capability::new("ptz", CapabilityValue::new_bool(true)).unwrap();
    let event = device
        .replace_capabilities(&clock, vec![capability.clone()])
        .unwrap();
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::DeviceUpdated { .. }
    ));
    assert_eq!(device.capabilities().len(), 1);
    assert_eq!(device.capabilities()[0], capability);

    let mut metadata = BTreeMap::new();
    metadata.insert("location".to_string(), "room-1".to_string());
    device.replace_metadata(&clock, metadata.clone()).unwrap();
    assert_eq!(device.metadata(), &metadata);
}

#[test]
fn device_new_rejects_unknown_kind() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let result = Device::new(
        &clock,
        tenant_id,
        device_id,
        cheetah_domain::Protocol::Gb28181,
        ProtocolIdentity::new("ext-1").unwrap(),
        "authority",
        "name",
        DeviceKind::Unknown,
        Vec::new(),
        BTreeMap::new(),
    );
    assert!(matches!(
        result,
        Err(cheetah_domain::DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn device_new_rejects_empty_name() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let result = Device::new(
        &clock,
        tenant_id,
        device_id,
        cheetah_domain::Protocol::Gb28181,
        ProtocolIdentity::new("ext-1").unwrap(),
        "authority",
        "",
        DeviceKind::Camera,
        Vec::new(),
        BTreeMap::new(),
    );
    assert!(matches!(
        result,
        Err(cheetah_domain::DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn channel_starts_with_given_status() {
    let (clock, _, tenant_id, device_id, channel_id) = setup();
    let channel = new_channel(&clock, tenant_id, device_id, channel_id);
    assert_eq!(channel.status(), ChannelStatus::Online);
    assert!(channel.enabled());
    assert_eq!(channel.kind(), ChannelKind::Video);
}

#[test]
fn channel_update_status_and_disable() {
    let (clock, _, tenant_id, device_id, channel_id) = setup();
    let mut channel = new_channel(&clock, tenant_id, device_id, channel_id);
    let event = channel.update_status(&clock, ChannelStatus::Fault);
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::ChannelOnlineChanged { .. }
    ));
    assert_eq!(channel.status(), ChannelStatus::Fault);
    let _ = channel.disable(&clock);
    assert!(!channel.enabled());
    let _ = channel.enable(&clock);
    assert!(channel.enabled());
}

#[test]
fn channel_update_rejects_empty_name() {
    let (clock, _, tenant_id, device_id, channel_id) = setup();
    let mut channel = new_channel(&clock, tenant_id, device_id, channel_id);
    let result = channel.update(
        &clock,
        None,
        Some("".to_string()),
        None,
        None,
        None,
        None,
        None,
    );
    assert!(matches!(
        result,
        Err(cheetah_domain::DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn channel_remove_emits_removed_event() {
    let (clock, _, tenant_id, device_id, channel_id) = setup();
    let channel = new_channel(&clock, tenant_id, device_id, channel_id);
    let event = channel.remove();
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::ChannelRemoved { .. }
    ));
}

#[test]
fn device_timestamps_use_injected_clock() {
    let (clock, _id_generator, tenant_id, device_id, _) = setup();
    let mut device = new_device(&clock, tenant_id, device_id);
    let created_at = device.created_at();
    clock.advance(DurationMs::from_millis(200));
    device.mark_online(&clock, None).unwrap();
    assert!(device.updated_at() > created_at);
}
