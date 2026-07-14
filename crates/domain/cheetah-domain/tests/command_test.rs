//! Command and IdempotencyScope value object tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, device_resource_ref, request_context,
};
use cheetah_domain::{
    CommandPayload, IdempotencyScope, MediaControl, MediaPurpose, Operation, PtzDirection,
};
use cheetah_signal_types::{IdGenerator, OwnerEpoch, UtcTimestamp};

fn create_command(
    id_generator: &InMemoryIdGenerator,
    payload: CommandPayload,
) -> cheetah_domain::Command {
    let clock = InMemoryClock::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let context = request_context(tenant_id, id_generator, &clock);
    let (operation, _) = Operation::new(
        id_generator,
        &clock,
        &context,
        "key",
        device_id,
        device_resource_ref(tenant_id, device_id),
        payload,
        None,
        OwnerEpoch::default(),
    )
    .unwrap();
    operation.command().clone()
}

#[test]
fn command_preserves_envelope_fields() {
    let id_generator = InMemoryIdGenerator::new();
    let command = create_command(
        &id_generator,
        CommandPayload::StartLive {
            media_session_id: id_generator.generate_media_session_id(),
            channel_id: id_generator.generate_channel_id(),
            media_node_id: id_generator.generate_node_id(),
            purpose: MediaPurpose::Live,
        },
    );
    assert!(!command.command_id().as_uuid().is_nil());
    assert!(!command.message_id().as_uuid().is_nil());
    assert!(!command.operation_id().as_uuid().is_nil());
    assert_eq!(command.idempotency_key(), "key");
    assert_eq!(command.kind(), "StartLive");
    assert_eq!(command.deadline(), None);
    assert_eq!(command.expected_owner_epoch(), OwnerEpoch::default());
}

#[test]
fn command_payload_kinds() {
    let id_generator = InMemoryIdGenerator::new();
    let live = CommandPayload::StartLive {
        media_session_id: id_generator.generate_media_session_id(),
        channel_id: id_generator.generate_channel_id(),
        media_node_id: id_generator.generate_node_id(),
        purpose: MediaPurpose::Live,
    };
    assert_eq!(live.kind(), "StartLive");

    let playback = CommandPayload::StartPlayback {
        media_session_id: id_generator.generate_media_session_id(),
        channel_id: id_generator.generate_channel_id(),
        media_node_id: id_generator.generate_node_id(),
        start_time: UtcTimestamp::default(),
        end_time: UtcTimestamp::default(),
        scale: 1.0,
    };
    assert_eq!(playback.kind(), "StartPlayback");

    let talk = CommandPayload::StartTalk {
        media_session_id: id_generator.generate_media_session_id(),
        channel_id: id_generator.generate_channel_id(),
        media_node_id: id_generator.generate_node_id(),
    };
    assert_eq!(talk.kind(), "StartTalk");

    let stop = CommandPayload::StopMediaSession {
        media_session_id: id_generator.generate_media_session_id(),
    };
    assert_eq!(stop.kind(), "StopMediaSession");

    let control = CommandPayload::ControlPlayback {
        media_session_id: id_generator.generate_media_session_id(),
        command: MediaControl::Pause,
    };
    assert_eq!(control.kind(), "ControlPlayback");

    let ptz = CommandPayload::Ptz {
        channel_id: id_generator.generate_channel_id(),
        direction: PtzDirection::Down,
        speed: 0.5,
    };
    assert_eq!(ptz.kind(), "Ptz");
}

#[test]
fn idempotency_scope_uses_target_and_principal() {
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let target = device_resource_ref(tenant_id, device_id);
    let scope = IdempotencyScope::new(tenant_id, "user-1", target.clone(), "req-1").unwrap();
    assert_eq!(scope.tenant_id, tenant_id);
    assert_eq!(scope.principal_id, "user-1");
    assert_eq!(scope.target, target);
    assert_eq!(scope.idempotency_key, "req-1");
}

#[test]
fn idempotency_scope_inequality_by_target() {
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_a = id_generator.generate_device_id();
    let device_b = id_generator.generate_device_id();
    let target_a = device_resource_ref(tenant_id, device_a);
    let target_b = device_resource_ref(tenant_id, device_b);
    let scope_a = IdempotencyScope::new(tenant_id, "p", target_a, "k").unwrap();
    let scope_b = IdempotencyScope::new(tenant_id, "p", target_b, "k").unwrap();
    assert_ne!(scope_a, scope_b);
}

#[test]
fn idempotency_scope_inequality_by_key() {
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let target = device_resource_ref(tenant_id, device_id);
    let scope_a = IdempotencyScope::new(tenant_id, "p", target.clone(), "k1").unwrap();
    let scope_b = IdempotencyScope::new(tenant_id, "p", target, "k2").unwrap();
    assert_ne!(scope_a, scope_b);
}
