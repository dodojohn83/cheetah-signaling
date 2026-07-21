//! Device service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{Connectivity, DeviceLifecycle, UnitOfWork};
use cheetah_signal_application::{
    CapabilityDto, CapabilityValueDto, ChannelDescriptor, MarkDeviceOfflineRequest,
    MarkDeviceOnlineRequest, RegisterDeviceRequest, ReplaceChannelCatalogRequest,
    RetireDeviceRequest, UpdateDeviceCapabilitiesRequest,
};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn device_service_register_update_and_retire() {
    let mut ctx = setup();
    let context = request_context(&ctx);

    let device = ctx
        .device_service
        .register_or_update_device(
            &context,
            &mut ctx.uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-1".to_string(),
                authority: Some("authority".to_string()),
                name: "camera-01".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;
    assert_eq!(device.lifecycle, DeviceLifecycle::Provisioning);

    let online = ctx
        .device_service
        .mark_device_online(
            &context,
            &mut ctx.uow,
            device.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();
    assert_eq!(online.lifecycle, DeviceLifecycle::Active);
    assert_eq!(online.connectivity, Connectivity::Online);

    let offline = ctx
        .device_service
        .mark_device_offline(
            &context,
            &mut ctx.uow,
            device.device_id,
            MarkDeviceOfflineRequest {
                reason: "maintenance".to_string(),
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(offline.connectivity, Connectivity::Offline { reason, .. } if reason == "maintenance")
    );

    let capabilities = vec![CapabilityDto {
        key: "ptz".to_string(),
        value: CapabilityValueDto::Boolean(true),
    }];
    let updated = ctx
        .device_service
        .update_device_capabilities(
            &context,
            &mut ctx.uow,
            device.device_id,
            device.revision,
            UpdateDeviceCapabilitiesRequest {
                capabilities: Some(capabilities),
                metadata: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.capabilities.len(), 1);

    let retired = ctx
        .device_service
        .retire_device(
            &context,
            &mut ctx.uow,
            device.device_id,
            RetireDeviceRequest {},
        )
        .await
        .unwrap();
    assert_eq!(retired.lifecycle, DeviceLifecycle::Retired);
}

#[tokio::test(flavor = "current_thread")]
async fn device_service_replace_channel_catalog() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = ctx
        .device_service
        .register_or_update_device(
            &context,
            &mut ctx.uow,
            RegisterDeviceRequest {
                protocol: "gb28181".to_string(),
                external_id: "ext-1".to_string(),
                authority: Some("authority".to_string()),
                name: "camera-01".to_string(),
                kind: "camera".to_string(),
                capabilities: None,
                metadata: None,
            },
        )
        .await
        .unwrap()
        .device;

    ctx.device_service
        .mark_device_online(
            &context,
            &mut ctx.uow,
            device.device_id,
            MarkDeviceOnlineRequest { reason: None },
        )
        .await
        .unwrap();

    let _ = ctx
        .device_service
        .replace_channel_catalog(
            &context,
            &mut ctx.uow,
            device.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![
                    ChannelDescriptor {
                        id: None,
                        name: "ch1".to_string(),
                        kind: "video".to_string(),
                        enabled: true,
                        status: Some("online".to_string()),
                        stream_profiles: Vec::new(),
                        ptz_capabilities: None,
                        metadata: None,
                    },
                    ChannelDescriptor {
                        id: None,
                        name: "ch2".to_string(),
                        kind: "audio".to_string(),
                        enabled: false,
                        status: Some("offline".to_string()),
                        stream_profiles: Vec::new(),
                        ptz_capabilities: None,
                        metadata: None,
                    },
                ],
            },
        )
        .await
        .unwrap();
    ctx.uow.commit().await.unwrap();
    let channels = ctx
        .uow
        .channel_repository()
        .list_by_device(ctx.tenant_id, device.device_id)
        .await
        .unwrap();
    assert_eq!(channels.len(), 2);
    assert_eq!(channels[0].name(), "ch1");
    assert_eq!(channels[1].name(), "ch2");
}
