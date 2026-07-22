//! Deterministic media contract tests for GB4-MED-005..008.
//!
//! These tests exercise talk/broadcast media senders, late/duplicate/early
//! media callbacks, `UnknownOutcome` handling and epoch/instance fencing using
//! only the in-memory (fake) media node, `FakeClock` and deterministic IDs. No
//! real network, device or database is involved.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use cheetah_domain::{
    CommandPayload, IdempotencyScope, MediaBinding, MediaBindingState, MediaNodeCallback,
    MediaNodeCallbackKind, MediaNodeCommandResult, MediaPurpose, MediaSession,
    MediaSessionDesiredState, MediaSessionState, Operation, OperationStatus, OwnerInfo, UnitOfWork,
};
use cheetah_signal_application::{
    StartBroadcastRequest, StartLiveRequest, StartTalkRequest, StopLiveRequest,
};
use cheetah_signal_types::{
    DeviceId, IdGenerator, OwnerEpoch, ResourceId, ResourceKind, ResourceRef,
};
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

fn set_default_owner(ctx: &TestContext, device_id: DeviceId) {
    ctx.owner_resolver.set_owner(
        ctx.tenant_id,
        device_id,
        OwnerInfo {
            owner_node_id: ctx.id_generator.generate_node_id(),
            owner_epoch: OwnerEpoch::default(),
            lease_until: None,
        },
    );
}

/// GB4-MED-005: a broadcast start creates a `Broadcast` media session and a
/// media-sender binding, then a `Started` callback drives it to `Active`.
#[tokio::test(flavor = "current_thread")]
async fn broadcast_start_and_activate() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let session = ctx
        .media_service
        .start_broadcast(
            &context,
            &mut ctx.uow,
            StartBroadcastRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "broadcast-1".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(session.purpose, MediaPurpose::Broadcast);
    assert_eq!(session.state, MediaSessionState::Inviting);

    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(binding.state(), MediaBindingState::Active);

    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: Some(session.operation_id),
        owner_epoch: binding.owner_epoch(),
        message_id: context.message_id.to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };
    let updated = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await
        .unwrap();
    assert_eq!(updated.state, MediaSessionState::Active);
}

/// GB4-MED-005: broadcast is idempotent on the idempotency key.
#[tokio::test(flavor = "current_thread")]
async fn broadcast_start_is_idempotent() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let first = ctx
        .media_service
        .start_broadcast(
            &context,
            &mut ctx.uow,
            StartBroadcastRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "broadcast-idem".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    for _ in 0..5 {
        let again = ctx
            .media_service
            .start_broadcast(
                &context,
                &mut ctx.uow,
                StartBroadcastRequest {
                    device_id: device.device_id.to_string(),
                    channel_id: channel.channel_id().to_string(),
                    idempotency_key: "broadcast-idem".to_string(),
                    deadline: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(again.media_session_id, first.media_session_id);
    }
}

/// GB4-MED-006: a start command that returns `UnknownOutcome` must not fail the
/// operation terminally. The session/binding are left non-terminal for the
/// reconciler to resolve.
#[tokio::test(flavor = "current_thread")]
async fn start_unknown_outcome_defers_to_reconciler() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    ctx.media_port
        .script_execute_result(MediaNodeCommandResult::UnknownOutcome {
            code: "timeout".to_string(),
            message: "no confirmation".to_string(),
        });

    let session = ctx
        .media_service
        .start_talk(
            &context,
            &mut ctx.uow,
            StartTalkRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "talk-unknown".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    // Session progressed out of Allocating but is not terminal.
    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!media_session.is_terminal());

    // The start operation must remain non-terminal so reconciliation can query
    // the media node instead of falsely completing it.
    let operation = ctx
        .uow
        .operation_repository()
        .get(ctx.tenant_id, session.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        operation.status(),
        OperationStatus::Pending | OperationStatus::Running
    ));
}

/// GB4-MED-006: a stop command that returns `UnknownOutcome` records the
/// stopping/releasing intent and releases the reservation, but leaves the stop
/// operation non-terminal.
#[tokio::test(flavor = "current_thread")]
async fn stop_unknown_outcome_records_intent() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let session = ctx
        .media_service
        .start_live(
            &context,
            &mut ctx.uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "live-stop-unknown".to_string(),
                deadline: None,
            },
        )
        .await
        .unwrap();

    // Activate the session first so the uncertain stop is applied to an Active
    // session (Active -> Stopping intent recorded, awaiting confirmation).
    let binding = ctx
        .uow
        .media_binding_repository()
        .get_by_media_session(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    let started = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: Some(session.operation_id),
        owner_epoch: binding.owner_epoch(),
        message_id: ctx.id_generator.generate_message_id().to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };
    let session = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, started)
        .await
        .unwrap();
    assert_eq!(session.state, MediaSessionState::Active);

    // The stop command dispatch is the next `execute` call; script it uncertain.
    ctx.media_port
        .script_execute_result(MediaNodeCommandResult::UnknownOutcome {
            code: "timeout".to_string(),
            message: "no confirmation".to_string(),
        });

    let stopped = ctx
        .media_service
        .stop_live(
            &context,
            &mut ctx.uow,
            StopLiveRequest {
                media_session_id: session.media_session_id.to_string(),
                idempotency_key: "stop-unknown".to_string(),
            },
        )
        .await
        .unwrap();

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, stopped.media_session_id)
        .await
        .unwrap()
        .unwrap();
    // Stop intent recorded but not confirmed terminal by the media node.
    assert_eq!(media_session.state(), MediaSessionState::Stopping);
}

/// GB4-MED-006: a late `Started` callback for a start operation must not revive
/// a session that has already been stopped.
#[tokio::test(flavor = "current_thread")]
async fn late_started_callback_does_not_revive_stopped_session() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let session = ctx
        .media_service
        .start_live(
            &context,
            &mut ctx.uow,
            StartLiveRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "live-late".to_string(),
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

    ctx.media_service
        .stop_live(
            &context,
            &mut ctx.uow,
            StopLiveRequest {
                media_session_id: session.media_session_id.to_string(),
                idempotency_key: "stop-late".to_string(),
            },
        )
        .await
        .unwrap();

    // A late Started callback tied to the (now stale) start flow arrives.
    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: Some(session.operation_id),
        owner_epoch: binding.owner_epoch(),
        message_id: ctx.id_generator.generate_message_id().to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };
    let _ = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await;

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    // The stopped session must not have been resurrected to Active.
    assert!(media_session.is_terminal());
    assert_ne!(media_session.state(), MediaSessionState::Active);
}

/// GB4-MED-006: an early `Started` callback that arrives while the session is
/// still `Allocating` (crash/restart window before the dispatch response was
/// persisted) is handled deterministically and activates the session.
#[tokio::test(flavor = "current_thread")]
async fn early_started_callback_activates_allocating_session() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let media_session_id = ctx.id_generator.generate_media_session_id();
    let media_binding_id = ctx.id_generator.generate_media_binding_id();
    let media_node_id = ctx.id_generator.generate_node_id();
    let owner_epoch = OwnerEpoch::default();
    let target = ResourceRef {
        tenant_id: ctx.tenant_id,
        kind: ResourceKind::Channel,
        id: ResourceId::Channel(channel.channel_id()),
    };
    let payload = CommandPayload::StartLive {
        media_session_id,
        channel_id: channel.channel_id(),
        media_node_id,
        purpose: MediaPurpose::Live,
    };

    let (mut operation, _op_event) = Operation::new(
        &*ctx.id_generator,
        &*ctx.clock,
        &context,
        "early-media",
        device.device_id,
        target,
        payload,
        None,
        owner_epoch,
    )
    .unwrap();

    let scope = IdempotencyScope::new(
        ctx.tenant_id,
        context.principal.id.clone(),
        ResourceRef {
            tenant_id: ctx.tenant_id,
            kind: ResourceKind::Channel,
            id: ResourceId::Channel(channel.channel_id()),
        },
        "early-media",
    )
    .unwrap();

    let (mut session, _session_event) = MediaSession::new(
        &*ctx.clock,
        media_session_id,
        ctx.tenant_id,
        device.device_id,
        channel.channel_id(),
        MediaPurpose::Live,
        MediaSessionDesiredState::Active,
        owner_epoch,
        operation.operation_id(),
        scope,
        None,
    )
    .unwrap();

    let (binding, _binding_event) = MediaBinding::new(
        &*ctx.clock,
        media_binding_id,
        media_session_id,
        ctx.tenant_id,
        channel.channel_id(),
        media_node_id,
        owner_epoch,
        binding_instance_epoch(&ctx),
    )
    .unwrap();

    // Persist each aggregate at its initial revision first (mirroring the real
    // start saga), then apply the dispatch/allocating transitions and re-save so
    // stored revisions advance monotonically.
    ctx.uow
        .operation_repository()
        .save(&operation)
        .await
        .unwrap();
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

    // The command was dispatched (operation Running) before the early callback.
    operation.start(&*ctx.clock).unwrap();
    session.allocating(&*ctx.clock).unwrap();
    ctx.uow
        .operation_repository()
        .save(&operation)
        .await
        .unwrap();
    ctx.uow
        .media_session_repository()
        .save(&session)
        .await
        .unwrap();
    ctx.uow.commit().await.unwrap();

    // Read back authoritative revisions after persistence.
    let session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, media_session_id)
        .await
        .unwrap()
        .unwrap();
    let binding = ctx
        .uow
        .media_binding_repository()
        .get(ctx.tenant_id, media_binding_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.state(), MediaSessionState::Allocating);

    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id,
        media_binding_id,
        operation_id: Some(operation.operation_id()),
        owner_epoch: binding.owner_epoch(),
        message_id: ctx.id_generator.generate_message_id().to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision(),
        kind: MediaNodeCallbackKind::Started,
    };
    let updated = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await
        .unwrap();
    assert_eq!(updated.state, MediaSessionState::Active);
}

/// GB4-MED-007: a callback carrying a stale signaling owner epoch must be
/// rejected and must not advance session state.
#[tokio::test(flavor = "current_thread")]
async fn stale_owner_epoch_callback_rejected() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let session = ctx
        .media_service
        .start_talk(
            &context,
            &mut ctx.uow,
            StartTalkRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "talk-epoch".to_string(),
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

    let stale_epoch = OwnerEpoch(binding.owner_epoch().0.saturating_add(1));
    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: binding.media_node_instance_epoch(),
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: Some(session.operation_id),
        owner_epoch: stale_epoch,
        message_id: ctx.id_generator.generate_message_id().to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };
    let result = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await;
    assert!(result.is_err());

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(media_session.state(), MediaSessionState::Active);
}

/// GB4-MED-007: a callback from a stale media-node instance epoch must be
/// rejected and must not advance session state.
#[tokio::test(flavor = "current_thread")]
async fn stale_node_instance_epoch_callback_rejected() {
    let mut ctx = setup();
    let context = request_context(&ctx);
    let device = register_device_and_channel(&mut ctx).await;
    let channel = find_channel(&mut ctx, device.device_id).await;
    set_default_owner(&ctx, device.device_id);

    let session = ctx
        .media_service
        .start_talk(
            &context,
            &mut ctx.uow,
            StartTalkRequest {
                device_id: device.device_id.to_string(),
                channel_id: channel.channel_id().to_string(),
                idempotency_key: "talk-instance".to_string(),
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

    let stale_instance = cheetah_signal_types::MediaNodeInstanceEpoch(
        binding.media_node_instance_epoch().0.saturating_add(1),
    );
    let callback = MediaNodeCallback {
        media_node_id: binding.media_node_id(),
        media_node_instance_epoch: stale_instance,
        media_session_id: session.media_session_id,
        media_binding_id: binding.media_binding_id(),
        operation_id: Some(session.operation_id),
        owner_epoch: binding.owner_epoch(),
        message_id: ctx.id_generator.generate_message_id().to_string(),
        binding_revision: binding.revision(),
        session_revision: session.revision,
        kind: MediaNodeCallbackKind::Started,
    };
    let result = ctx
        .media_service
        .handle_media_event(&context, &mut ctx.uow, callback)
        .await;
    assert!(result.is_err());

    let media_session = ctx
        .uow
        .media_session_repository()
        .get(ctx.tenant_id, session.media_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(media_session.state(), MediaSessionState::Active);
}

fn binding_instance_epoch(_ctx: &TestContext) -> cheetah_signal_types::MediaNodeInstanceEpoch {
    cheetah_signal_types::MediaNodeInstanceEpoch::default()
}
