//! MediaSessionReconciler tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{
    IdempotencyScope, MediaBinding, MediaBindingState, MediaPurpose, MediaSession,
    MediaSessionDesiredState, MediaSessionError, OwnerInfo, UnitOfWork,
};
use cheetah_signal_application::MediaSessionReconciler;
use cheetah_signal_types::{IdGenerator, OwnerEpoch, ResourceId, ResourceKind, ResourceRef};
use common::*;

#[tokio::test(flavor = "current_thread")]
async fn media_session_reconciler_releases_live_binding_for_terminal_session() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = ctx
        .uow
        .channel_repository()
        .list_by_device(ctx.tenant_id, device.device_id)
        .await
        .unwrap()
        .pop()
        .unwrap();

    let media_session_id = ctx.id_generator.generate_media_session_id();
    let media_binding_id = ctx.id_generator.generate_media_binding_id();
    let operation_id = ctx.id_generator.generate_operation_id();
    let media_node_id = ctx.id_generator.generate_node_id();
    let owner_epoch = OwnerEpoch(1);
    let media_node_instance_epoch = ctx.id_generator.generate_media_node_instance_epoch();

    let scope = IdempotencyScope::new(
        ctx.tenant_id,
        context.principal.id.clone(),
        ResourceRef {
            tenant_id: ctx.tenant_id,
            kind: ResourceKind::Channel,
            id: ResourceId::Channel(channel.channel_id()),
        },
        "terminal-live-binding".to_string(),
    )
    .unwrap();

    let clock = ctx.clock.as_ref();
    let (mut session, _) = MediaSession::new(
        clock,
        media_session_id,
        ctx.tenant_id,
        device.device_id,
        channel.channel_id(),
        MediaPurpose::Live,
        MediaSessionDesiredState::Active,
        owner_epoch,
        operation_id,
        scope,
        None,
    )
    .unwrap();

    session
        .failed(MediaSessionError::new("test", "terminal"), clock)
        .unwrap();

    let (binding, _) = MediaBinding::new(
        clock,
        media_binding_id,
        media_session_id,
        ctx.tenant_id,
        channel.channel_id(),
        media_node_id,
        owner_epoch,
        media_node_instance_epoch,
    )
    .unwrap();

    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device.device_id,
        OwnerInfo {
            owner_node_id: ctx.source_node_id,
            owner_epoch,
            lease_until: None,
        },
    );

    ctx.uow
        .media_session_repository()
        .save(&session)
        .await
        .unwrap();
    ctx.uow
        .media_binding_repository()
        .save(&binding)
        .await
        .unwrap();

    let reconciler = MediaSessionReconciler::new(ctx.clock.clone(), ctx.id_generator.clone(), 1000);
    let report = reconciler.reconcile(&context, &mut ctx.uow).await.unwrap();

    ctx.uow.commit().await.unwrap();

    assert_eq!(report.scanned, 1);
    assert_eq!(report.released, 1);
    assert_eq!(report.reservations_to_release, vec![media_binding_id]);

    let binding_after = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, media_session_id)
        .await
        .unwrap()
        .expect("binding should exist");
    assert_eq!(binding_after.state(), MediaBindingState::Released);

    let session_after = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, media_session_id)
        .await
        .unwrap()
        .expect("session should exist");
    assert!(session_after.is_terminal());
}
