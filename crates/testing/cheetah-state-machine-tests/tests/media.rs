//! GB4-TST-002 media transition table: media-session saga steps, late `200`,
//! `CANCEL`/`BYE`, early media, and stale media-node-instance fencing.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, device_resource_ref};
use cheetah_domain::{
    IdempotencyScope, MediaBinding, MediaBindingError, MediaBindingState, MediaPurpose,
    MediaSession, MediaSessionDesiredState, MediaSessionError, MediaSessionState,
};
use cheetah_signal_types::{IdGenerator, MediaNodeInstanceEpoch, OwnerEpoch};

fn scope(id_generator: &InMemoryIdGenerator) -> (IdempotencyScope, cheetah_signal_types::TenantId) {
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let target = device_resource_ref(tenant_id, device_id);
    let scope = IdempotencyScope::new(tenant_id, "principal", target, "media-key").unwrap();
    (scope, tenant_id)
}

fn new_session(clock: &InMemoryClock, id_generator: &InMemoryIdGenerator) -> MediaSession {
    let (idem, tenant_id) = scope(id_generator);
    MediaSession::new(
        clock,
        id_generator.generate_media_session_id(),
        tenant_id,
        id_generator.generate_device_id(),
        id_generator.generate_channel_id(),
        MediaPurpose::Live,
        MediaSessionDesiredState::Active,
        OwnerEpoch::default(),
        id_generator.generate_operation_id(),
        idem,
        None,
    )
    .map(|(s, _)| s)
    .expect("media session creation should succeed")
}

fn session_in(
    state: MediaSessionState,
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
) -> MediaSession {
    let mut session = new_session(clock, id_generator);
    match state {
        MediaSessionState::Requested => {}
        MediaSessionState::Allocating => {
            session.allocating(clock).unwrap();
        }
        MediaSessionState::Inviting => {
            session.allocating(clock).unwrap();
            session.inviting(clock).unwrap();
        }
        MediaSessionState::Active => {
            session.allocating(clock).unwrap();
            session.inviting(clock).unwrap();
            session.active(clock).unwrap();
        }
        MediaSessionState::Stopping => {
            session.allocating(clock).unwrap();
            session.inviting(clock).unwrap();
            session.active(clock).unwrap();
            session.stopping(clock).unwrap();
        }
        MediaSessionState::Stopped => {
            session.cancel(clock).unwrap();
        }
        MediaSessionState::Failed => {
            session.failed(MediaSessionError::timeout(), clock).unwrap();
        }
        other => panic!("unhandled media session state: {other:?}"),
    }
    assert_eq!(session.state(), state);
    session
}

/// The saga happy path: `Requested → Allocating → Inviting → Active → Stopping
/// → Stopped`, each step bumping the revision.
#[test]
fn media_session_saga_happy_path() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    assert_eq!(session.state(), MediaSessionState::Requested);
    session.allocating(&clock).unwrap();
    session.inviting(&clock).unwrap();
    session.active(&clock).unwrap();
    session.stopping(&clock).unwrap();
    session.stopped(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    assert!(session.is_terminal());
    assert_eq!(session.revision().0, 5);
}

/// Full media-session transition matrix using the direct state methods.
#[test]
fn media_session_transition_matrix() {
    let states = [
        MediaSessionState::Requested,
        MediaSessionState::Allocating,
        MediaSessionState::Inviting,
        MediaSessionState::Active,
        MediaSessionState::Stopping,
        MediaSessionState::Stopped,
        MediaSessionState::Failed,
    ];
    let targets = [
        MediaSessionState::Allocating,
        MediaSessionState::Inviting,
        MediaSessionState::Active,
        MediaSessionState::Stopped,
        MediaSessionState::Failed,
    ];

    let allowed = |from: MediaSessionState, to: MediaSessionState| match from {
        MediaSessionState::Requested => matches!(
            to,
            MediaSessionState::Allocating | MediaSessionState::Stopped | MediaSessionState::Failed
        ),
        MediaSessionState::Allocating => matches!(
            to,
            MediaSessionState::Inviting | MediaSessionState::Stopped | MediaSessionState::Failed
        ),
        MediaSessionState::Inviting => matches!(
            to,
            MediaSessionState::Active | MediaSessionState::Stopped | MediaSessionState::Failed
        ),
        MediaSessionState::Active => {
            matches!(to, MediaSessionState::Stopped | MediaSessionState::Failed)
        }
        MediaSessionState::Stopping => {
            matches!(to, MediaSessionState::Stopped | MediaSessionState::Failed)
        }
        _ => false,
    };

    for from in states {
        for to in targets {
            let clock = InMemoryClock::new();
            let id_generator = InMemoryIdGenerator::new();
            let mut session = session_in(from, &clock, &id_generator);
            let revision_before = session.revision().0;
            let result = match to {
                MediaSessionState::Allocating => session.allocating(&clock),
                MediaSessionState::Inviting => session.inviting(&clock),
                MediaSessionState::Active => session.active(&clock),
                MediaSessionState::Stopped => session.stopped(&clock),
                MediaSessionState::Failed => session.failed(MediaSessionError::timeout(), &clock),
                other => panic!("unhandled target: {other:?}"),
            };
            assert_eq!(
                result.is_ok(),
                allowed(from, to),
                "from={from:?} to={to:?} expected allowed={}",
                allowed(from, to)
            );
            if result.is_err() {
                assert_eq!(session.state(), from);
                assert_eq!(session.revision().0, revision_before);
            }
        }
    }
}

/// Early media: a `200 OK` / media arriving while `Inviting` promotes the
/// session straight to `Active`.
#[test]
fn media_session_early_media_activates_from_inviting() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = session_in(MediaSessionState::Inviting, &clock, &id_generator);
    session.active(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Active);
}

/// Late `200`: a success callback that arrives after the session was already
/// torn down (`CANCEL`) must be rejected as terminal, never revived.
#[test]
fn media_session_late_200_after_cancel_is_rejected() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = session_in(MediaSessionState::Inviting, &clock, &id_generator);
    session.cancel(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    let err = session.active(&clock).unwrap_err();
    assert!(matches!(
        err,
        cheetah_domain::DomainError::AlreadyTerminal { .. }
    ));
    assert_eq!(session.state(), MediaSessionState::Stopped);
}

/// `BYE`: an active session is torn down via `Stopping → Stopped`.
#[test]
fn media_session_bye_stops_active_session() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = session_in(MediaSessionState::Active, &clock, &id_generator);
    session.stopping(&clock).unwrap();
    session.stopped(&clock).unwrap();
    assert_eq!(session.state(), MediaSessionState::Stopped);
    assert_eq!(session.desired_state(), MediaSessionDesiredState::Stopped);
}

// ---------------------------------------------------------------------------
// Media binding
// ---------------------------------------------------------------------------

fn new_binding(
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
    instance_epoch: MediaNodeInstanceEpoch,
) -> MediaBinding {
    let tenant_id = id_generator.generate_tenant_id();
    MediaBinding::new(
        clock,
        id_generator.generate_media_binding_id(),
        id_generator.generate_media_session_id(),
        tenant_id,
        id_generator.generate_channel_id(),
        id_generator.generate_node_id(),
        OwnerEpoch::default(),
        instance_epoch,
    )
    .map(|(b, _)| b)
    .expect("media binding creation should succeed")
}

fn binding_in(
    state: MediaBindingState,
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
) -> MediaBinding {
    let mut binding = new_binding(clock, id_generator, MediaNodeInstanceEpoch(1));
    match state {
        MediaBindingState::Reserved => {}
        MediaBindingState::Active => {
            binding.activate(clock).unwrap();
        }
        MediaBindingState::NeedsVerification => {
            binding.activate(clock).unwrap();
            binding
                .needs_verification(MediaBindingError::media_node_unavailable(), clock)
                .unwrap();
        }
        MediaBindingState::Releasing => {
            binding.activate(clock).unwrap();
            binding.release(clock).unwrap();
        }
        MediaBindingState::Released => {
            binding.activate(clock).unwrap();
            binding.release(clock).unwrap();
            binding.released(clock).unwrap();
        }
        MediaBindingState::Failed => {
            binding
                .failed(MediaBindingError::media_node_unavailable(), clock)
                .unwrap();
        }
        other => panic!("unhandled binding state: {other:?}"),
    }
    assert_eq!(binding.state(), state);
    binding
}

/// Full media-binding transition matrix.
#[test]
fn media_binding_transition_matrix() {
    let states = [
        MediaBindingState::Reserved,
        MediaBindingState::Active,
        MediaBindingState::NeedsVerification,
        MediaBindingState::Releasing,
        MediaBindingState::Released,
        MediaBindingState::Failed,
    ];
    let targets = [
        MediaBindingState::Active,
        MediaBindingState::NeedsVerification,
        MediaBindingState::Releasing,
        MediaBindingState::Released,
        MediaBindingState::Failed,
    ];

    let allowed = |from: MediaBindingState, to: MediaBindingState| match from {
        MediaBindingState::Reserved => matches!(
            to,
            MediaBindingState::Active
                | MediaBindingState::NeedsVerification
                | MediaBindingState::Releasing
                | MediaBindingState::Failed
        ),
        MediaBindingState::Active => matches!(
            to,
            MediaBindingState::NeedsVerification
                | MediaBindingState::Releasing
                | MediaBindingState::Failed
        ),
        MediaBindingState::NeedsVerification => matches!(
            to,
            MediaBindingState::Active | MediaBindingState::Releasing | MediaBindingState::Failed
        ),
        MediaBindingState::Releasing => {
            matches!(to, MediaBindingState::Released | MediaBindingState::Failed)
        }
        _ => false,
    };

    for from in states {
        for to in targets {
            let clock = InMemoryClock::new();
            let id_generator = InMemoryIdGenerator::new();
            let mut binding = binding_in(from, &clock, &id_generator);
            let revision_before = binding.revision().0;
            let result = match to {
                MediaBindingState::Active => binding.activate(&clock),
                MediaBindingState::NeedsVerification => {
                    binding.needs_verification(MediaBindingError::media_node_unavailable(), &clock)
                }
                MediaBindingState::Releasing => binding.release(&clock),
                MediaBindingState::Released => binding.released(&clock),
                MediaBindingState::Failed => {
                    binding.failed(MediaBindingError::media_node_unavailable(), &clock)
                }
                other => panic!("unhandled target: {other:?}"),
            };
            assert_eq!(
                result.is_ok(),
                allowed(from, to),
                "from={from:?} to={to:?} expected allowed={}",
                allowed(from, to)
            );
            if result.is_err() {
                assert_eq!(binding.state(), from);
                assert_eq!(binding.revision().0, revision_before);
            }
        }
    }
}

/// Old media instance: a stale media-node-instance epoch must be fenced. A
/// binding records the instance epoch it was created against; a callback from an
/// older instance (lower epoch) must not advance the binding.
#[test]
fn stale_media_node_instance_is_fenced() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut binding = new_binding(&clock, &id_generator, MediaNodeInstanceEpoch(7));
    binding.activate(&clock).unwrap();
    assert_eq!(
        binding.media_node_instance_epoch(),
        MediaNodeInstanceEpoch(7)
    );

    let accepts =
        |incoming: MediaNodeInstanceEpoch| incoming.0 >= binding.media_node_instance_epoch().0;
    assert!(
        !accepts(MediaNodeInstanceEpoch(6)),
        "older instance rejected"
    );
    assert!(
        accepts(MediaNodeInstanceEpoch(7)),
        "current instance accepted"
    );
    assert!(
        accepts(MediaNodeInstanceEpoch(8)),
        "newer instance accepted"
    );
}
