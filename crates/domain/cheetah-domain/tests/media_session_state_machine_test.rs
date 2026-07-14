//! MediaSession state machine and lifecycle tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, media_session_resource_ref};
use cheetah_domain::{
    IdempotencyScope, MediaBinding, MediaBindingError, MediaPurpose, MediaSession,
    MediaSessionDesiredState, MediaSessionError, MediaSessionState,
};
use cheetah_signal_types::{
    ChannelId, DeviceId, DurationMs, IdGenerator, MediaSessionId, OwnerEpoch, TenantId,
};

fn setup() -> (
    InMemoryClock,
    InMemoryIdGenerator,
    TenantId,
    DeviceId,
    ChannelId,
    MediaSessionId,
) {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let channel_id = id_generator.generate_channel_id();
    let media_session_id = id_generator.generate_media_session_id();
    (
        clock,
        id_generator,
        tenant_id,
        device_id,
        channel_id,
        media_session_id,
    )
}

fn new_session(
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
    tenant_id: TenantId,
    device_id: DeviceId,
    channel_id: ChannelId,
    media_session_id: MediaSessionId,
    purpose: MediaPurpose,
) -> MediaSession {
    let scope = IdempotencyScope::new(
        tenant_id,
        "principal",
        media_session_resource_ref(tenant_id, media_session_id),
        "key",
    )
    .unwrap();
    let operation_id = id_generator.generate_operation_id();
    MediaSession::new(
        clock,
        media_session_id,
        tenant_id,
        device_id,
        channel_id,
        purpose,
        MediaSessionDesiredState::Active,
        OwnerEpoch::default(),
        operation_id,
        scope,
        None,
    )
    .map(|(session, _)| session)
    .expect("media session creation should succeed")
}

#[test]
fn media_session_starts_requested() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    assert_eq!(session.state(), MediaSessionState::Requested);
    assert_eq!(session.desired_state(), MediaSessionDesiredState::Active);
    assert_eq!(session.purpose(), MediaPurpose::Live);
}

#[test]
fn media_session_transitions_to_allocating_inviting_active() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.allocating(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Allocating);
    session.inviting(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Inviting);
    session.active(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Active);
    assert_eq!(session.revision().0, 3);
}

#[test]
fn media_session_stops_from_active() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.allocating(&clock).unwrap();
    session.inviting(&clock).unwrap();
    session.active(&clock).unwrap();
    session.stopping(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopping);
    assert_eq!(session.desired_state(), MediaSessionDesiredState::Stopped);
    session.stopped(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    assert!(session.state().is_terminal());
}

#[test]
fn media_session_stops_from_requested() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.stop(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    assert_eq!(session.desired_state(), MediaSessionDesiredState::Stopped);
}

#[test]
fn media_session_fails_from_allocating() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.allocating(&clock).unwrap();
    session
        .failed(MediaSessionError::timeout(), &clock)
        .unwrap();
    assert_eq!(session.state(), MediaSessionState::Failed);
    assert!(session.error().is_some());
}

#[test]
fn media_session_cancels_from_inviting() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.allocating(&clock).unwrap();
    session.inviting(&clock).unwrap();
    session.cancel(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    assert_eq!(session.desired_state(), MediaSessionDesiredState::Stopped);
}

#[test]
fn media_session_timeout_is_failed() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Playback,
    );
    session.timeout(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Failed);
    assert_eq!(session.purpose(), MediaPurpose::Playback);
}

#[test]
fn media_session_terminal_is_irreversible() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.stop(&clock).unwrap();
    let err = session.active(&clock).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::AlreadyTerminal { .. }
    ));
}

#[test]
fn media_session_invalid_transition_requested_to_active() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    let err = session.active(&clock).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::InvalidTransition { .. }
    ));
}

#[test]
fn media_session_revision_and_timestamps_use_clock() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    let created_at = session.created_at();
    clock.advance(DurationMs::from_millis(50));
    session.allocating(&clock).unwrap();
    assert!(session.updated_at() > created_at);
    assert_eq!(session.revision().0, 1);
}

#[test]
fn media_session_new_rejects_nil_session_id() {
    let (clock, id_generator, tenant_id, device_id, channel_id, _session_id) = setup();
    let session_id = MediaSessionId::from_uuid(uuid::Uuid::nil());
    let scope = IdempotencyScope::new(
        tenant_id,
        "principal",
        media_session_resource_ref(tenant_id, session_id),
        "key",
    )
    .unwrap();
    let result = MediaSession::new(
        &clock,
        session_id,
        tenant_id,
        device_id,
        channel_id,
        MediaPurpose::Live,
        MediaSessionDesiredState::Active,
        OwnerEpoch::default(),
        id_generator.generate_operation_id(),
        scope,
        None,
    );
    assert!(matches!(
        result,
        Err(cheetah_domain::DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn media_binding_lifecycle_and_session_references() {
    let (clock, id_generator, tenant_id, device_id, channel_id, session_id) = setup();
    let mut session = new_session(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        channel_id,
        session_id,
        MediaPurpose::Live,
    );
    session.allocating(&clock).unwrap();

    let media_binding_id = id_generator.generate_media_binding_id();
    let media_node_id = id_generator.generate_node_id();
    let media_node_instance_epoch = id_generator.generate_media_node_instance_epoch();
    let (mut binding, _) = MediaBinding::new(
        &clock,
        media_binding_id,
        session_id,
        tenant_id,
        channel_id,
        media_node_id,
        OwnerEpoch::default(),
        media_node_instance_epoch,
    )
    .unwrap();
    assert_eq!(binding.media_session_id(), session_id);
    assert_eq!(binding.media_binding_id(), media_binding_id);
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Reserved);

    binding.activate(&clock).unwrap();
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Active);
    binding.release(&clock).unwrap();
    assert_eq!(
        binding.state(),
        cheetah_domain::MediaBindingState::Releasing
    );
    binding.released(&clock).unwrap();
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Released);
    assert!(binding.state().is_terminal());

    let err = binding.activate(&clock).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::AlreadyTerminal { .. }
    ));
}

#[test]
fn media_binding_fails_from_reserved() {
    let (clock, id_generator, tenant_id, _device_id, channel_id, session_id) = setup();
    let media_binding_id = id_generator.generate_media_binding_id();
    let (mut binding, _) = MediaBinding::new(
        &clock,
        media_binding_id,
        session_id,
        tenant_id,
        channel_id,
        id_generator.generate_node_id(),
        OwnerEpoch::default(),
        id_generator.generate_media_node_instance_epoch(),
    )
    .unwrap();
    binding
        .failed(MediaBindingError::media_node_unavailable(), &clock)
        .unwrap();
    assert_eq!(binding.state(), cheetah_domain::MediaBindingState::Failed);
    assert_eq!(binding.error().unwrap().code(), "media_node_unavailable");
}

#[test]
fn media_binding_reserved_to_released_is_invalid() {
    let (clock, id_generator, tenant_id, _device_id, channel_id, session_id) = setup();
    let media_binding_id = id_generator.generate_media_binding_id();
    let (mut binding, _) = MediaBinding::new(
        &clock,
        media_binding_id,
        session_id,
        tenant_id,
        channel_id,
        id_generator.generate_node_id(),
        OwnerEpoch::default(),
        id_generator.generate_media_node_instance_epoch(),
    )
    .unwrap();
    let err = binding.released(&clock).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::InvalidTransition { .. }
    ));
}
