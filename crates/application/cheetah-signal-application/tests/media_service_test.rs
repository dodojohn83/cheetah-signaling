//! Media service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{
    MediaNodeCallback, MediaNodeCallbackKind, MediaNodeSessionRef, MediaPurpose, OwnerInfo,
    UnitOfWork,
};
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

#[tokio::test(flavor = "current_thread")]
async fn media_service_callback_transitions_to_active() {
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
        .start_playback(
            &context,
            &mut ctx.uow,
            StartPlaybackRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                start_time: "2024-01-01T00:00:00Z".to_string(),
                end_time: "2024-01-01T01:00:00Z".to_string(),
                scale: 1.0,
                idempotency_key: "playback-callback-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.state, cheetah_domain::MediaSessionState::Inviting);

    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();

    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: session.operation_id,
        owner_epoch: binding.owner_epoch(),
        message_id: context.message_id.to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };

    let session = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await
        .unwrap();
    assert_eq!(session.state, cheetah_domain::MediaSessionState::Active);
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_reconcile_releases_stopped_binding() {
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
                idempotency_key: "reconcile-stop-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    let mut media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    media_session.stop(&*ctx.clock).unwrap();
    ctx.uow
        .media_session_repository()
        .save(&media_session)
        .await
        .unwrap();

    let report = ctx
        .media_service
        .reconcile(&context, &mut ctx.uow)
        .await
        .unwrap();

    assert_eq!(report.missing_released, 1);

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        media_session.state(),
        cheetah_domain::MediaSessionState::Stopped
    );
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Released);
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_reconcile_fails_missing_active_session() {
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
                idempotency_key: "reconcile-missing-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();

    ctx.media_port
        .set_node_sessions(ctx.tenant_id, binding.media_node_id(), Vec::new());

    let report = ctx
        .media_service
        .reconcile(&context, &mut ctx.uow)
        .await
        .unwrap();

    assert_eq!(report.nodes_scanned, 1);
    assert_eq!(report.missing_failed, 1);

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        media_session.state(),
        cheetah_domain::MediaSessionState::Failed
    );
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Failed);
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_reconcile_handles_mid_release_binding() {
    use cheetah_domain::{MediaBindingState, MediaSessionState};

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
                idempotency_key: "reconcile-mid-release-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    // Simulate an in-flight stop: session is Stopping, binding is Releasing.
    let mut media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let mut binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();

    let clock = ctx.clock.as_ref();
    media_session.active(clock).unwrap();
    media_session.stopping(clock).unwrap();
    binding.release(clock).unwrap();

    ctx.uow
        .media_session_repository()
        .save(&media_session)
        .await
        .unwrap();
    ctx.uow
        .media_binding_repository()
        .save(&binding)
        .await
        .unwrap();
    ctx.uow.commit().await.unwrap();

    // Reconcile should not fail on the Releasing binding and should complete release.
    let report = ctx
        .media_service
        .reconcile(&context, &mut ctx.uow)
        .await
        .unwrap();

    assert_eq!(report.missing_released, 1);

    let media_session_after = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding_after = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(media_session_after.state(), MediaSessionState::Stopped);
    assert_eq!(binding_after.state(), MediaBindingState::Released);
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_reconcile_fails_sessions_on_deregistered_node() {
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
                idempotency_key: "reconcile-deregistered-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    let report = ctx
        .media_service
        .reconcile(&context, &mut ctx.uow)
        .await
        .unwrap();

    assert_eq!(report.nodes_scanned, 0);
    assert_eq!(report.missing_failed, 1);

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        media_session.state(),
        cheetah_domain::MediaSessionState::Failed
    );
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Failed);
}

#[tokio::test(flavor = "current_thread")]
async fn media_service_reconcile_converges_active_session() {
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
                idempotency_key: "reconcile-converge-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();

    let reported = MediaNodeSessionRef {
        media_session_id: session.media_session_id,
        device_id: Some(device.device_id),
        channel_id: Some(channel.channel_id()),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
    };
    ctx.media_port
        .set_node_sessions(ctx.tenant_id, binding.media_node_id(), vec![reported]);

    let report = ctx
        .media_service
        .reconcile(&context, &mut ctx.uow)
        .await
        .unwrap();

    assert_eq!(report.nodes_scanned, 1);
    assert_eq!(report.sessions_found, 1);
    assert_eq!(report.missing_failed, 0);

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        media_session.state(),
        cheetah_domain::MediaSessionState::Active
    );
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Active);
}
