//! Media service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{MediaPurpose, OwnerInfo, UnitOfWork};
use cheetah_signal_application::{
    ChannelDescriptor, ControlPlaybackRequest, MediaControlDto, RegisterDeviceRequest,
    ReplaceChannelCatalogRequest, StartLiveRequest, StartPlaybackRequest, StartTalkRequest,
    StopLiveRequest,
};
use cheetah_signal_types::{Clock, DeviceId, IdGenerator, OwnerEpoch};
use common::*;

async fn find_channel(ctx: &mut TestContext, device_id: DeviceId) -> cheetah_domain::Channel {
    ctx.uow
        .channel_repository()
        .list_by_device(ctx.tenant_id, device_id)
        .await
        .unwrap()
        .pop()
        .unwrap()
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_start_live_and_stop_idempotent() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let session = ctx
        .media_service
        .start_live(
            &context,
            &mut ctx.uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "live-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    for _ in 0..99 {
        let same = ctx
            .media_service
            .start_live(
                &context,
                &mut ctx.uow,
                StartLiveRequest {
                    device_id: device.device_id.to_string(),
                    channel_id: channel.channel_id().to_string(),
                    idempotency_key: "live-1".to_string(),
                    deadline: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(same.media_session_id, session.media_session_id);
    }

    let stopped = ctx
        .media_service
        .stop_live(
            &context,
            &mut ctx.uow,
            StopLiveRequest {
                media_session_id: session.media_session_id.to_string(),
                idempotency_key: "stop-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.media_session_id, session.media_session_id);

    for _ in 0..10 {
        let same = ctx
            .media_service
            .stop_live(
                &context,
                &mut ctx.uow,
                StopLiveRequest {
                    media_session_id: session.media_session_id.to_string(),
                    idempotency_key: "stop-1".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(same.media_session_id, session.media_session_id);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_start_playback_and_control() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let now = ctx.clock.now_wall().to_string();
    let session = ctx
        .media_service
        .start_playback(
            &context,
            &mut ctx.uow,
            StartPlaybackRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                start_time: now.clone(),
                end_time: now,
                scale: 1.0,
                idempotency_key: "playback-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.purpose, MediaPurpose::Playback);

    let mut media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    media_session.inviting(&*ctx.clock).unwrap();
    media_session.active(&*ctx.clock).unwrap();
    ctx.uow
        .media_session_repository()
        .save(&media_session)
        .await
        .unwrap();

    let operation = ctx
        .media_service
        .control_playback(
            &context,
            &mut ctx.uow,
            ControlPlaybackRequest {
                media_session_id: session.media_session_id.to_string(),
                command: MediaControlDto::Play,
                idempotency_key: "control-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(operation.kind, "ControlPlayback");

    for _ in 0..5 {
        let same = ctx
            .media_service
            .control_playback(
                &context,
                &mut ctx.uow,
                ControlPlaybackRequest {
                    media_session_id: session.media_session_id.to_string(),
                    command: MediaControlDto::Play,
                    idempotency_key: "control-1".to_string(),
                },
            )
            .await
            .unwrap();
        assert_eq!(same.operation_id, operation.operation_id);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_start_talk_and_outbox_events() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let session = ctx
        .media_service
        .start_talk(
            &context,
            &mut ctx.uow,
            StartTalkRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "talk-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.purpose, MediaPurpose::Talk);

    let committed = ctx.uow.committed();
    let events = committed
        .outbox
        .iter()
        .map(|e| e.event.payload.clone())
        .collect::<Vec<_>>();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, cheetah_domain::DomainEvent::OperationSubmitted { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, cheetah_domain::DomainEvent::MediaSessionCreated { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, cheetah_domain::DomainEvent::MediaBindingCreated { .. }))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_rejects_inactive_device() {
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
        .replace_channel_catalog(
            &context,
            &mut ctx.uow,
            device.device_id,
            ReplaceChannelCatalogRequest {
                channels: vec![ChannelDescriptor {
                    id: None,
                    name: "ch1".to_string(),
                    kind: "video".to_string(),
                    enabled: true,
                    status: Some("online".to_string()),
                    stream_profiles: Vec::new(),
                    ptz_capabilities: None,
                    metadata: None,
                }],
            },
        )
        .await
        .unwrap();

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device.device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );

    let channel = ctx
        .uow
        .channel_repository()
        .list_by_device(ctx.tenant_id, device.device_id)
        .await
        .unwrap()
        .pop()
        .unwrap();

    let result = ctx
        .media_service
        .start_live(
            &context,
            &mut ctx.uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "live-1".to_string(),
                deadline: None,
            },
        )
        .await;
    assert!(result.is_err());
}
