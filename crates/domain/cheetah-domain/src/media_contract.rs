//! Reusable behavioral contract for [`MediaPort`] implementations.
//!
//! GB4-MED-008 requires that the deterministic fake media node and any real
//! media node expose the *same* observable control-plane contract. This module
//! defines a single suite, [`run_media_port_contract`], that exercises the
//! port's reserve / execute / release / query surface and asserts the
//! invariants every conforming implementation must satisfy:
//!
//! * `reserve_*` returns a reservation bound to a concrete media node and a
//!   non-zero contract version, and `reserve_broadcast` selects a media-sender
//!   capable node (delegating to talk semantics by default).
//! * `execute` of a well-formed start command reports either `Accepted`
//!   (asynchronous, awaiting callback) or `Completed` (synchronous) — never a
//!   spurious `Failed`/`UnknownOutcome` for a valid request.
//! * `execute` of a stop command converges (`Completed`/`Accepted`).
//! * `release` is idempotent: releasing a binding twice is harmless.
//! * `list_nodes` / `list_sessions` are queryable without error.
//!
//! The suite is transport-agnostic: it drives only the typed
//! `cheetah.media.v1` boundary and never touches media payloads or ports. The
//! fake-backed test runs in CI; a real-backed system test can call the same
//! function against a live media node when one is provisioned.
//!
//! This is test-support code: it asserts invariants by panicking on the first
//! violation, so `expect`/`assert` are intentional here.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::missing_panics_doc
)]

use crate::{
    CommandPayload, MediaNodeCommand, MediaNodeCommandResult, MediaPort, MediaPurpose,
    MediaRequirements,
};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, IdGenerator, MediaBindingId, MediaSessionId, OperationId,
    PageRequest, TenantId,
};

/// Identifiers used to drive a single contract run.
///
/// A fresh set of identifiers is generated per run so the suite can be executed
/// repeatedly against the same port without colliding with prior reservations.
#[derive(Clone, Debug)]
pub struct MediaPortContractIds {
    /// Tenant that owns the exercised resources.
    pub tenant_id: TenantId,
    /// Device the media session targets.
    pub device_id: DeviceId,
    /// Channel the media session targets.
    pub channel_id: ChannelId,
}

impl MediaPortContractIds {
    /// Generates a deterministic-per-generator set of identifiers.
    pub fn generate(id_generator: &dyn IdGenerator) -> Self {
        Self {
            tenant_id: id_generator.generate_tenant_id(),
            device_id: id_generator.generate_device_id(),
            channel_id: id_generator.generate_channel_id(),
        }
    }
}

fn requirements(operation: &str, require_media_sender: bool) -> MediaRequirements {
    MediaRequirements {
        protocol: "gb28181".to_string(),
        operation: operation.to_string(),
        session_type: operation.to_string(),
        require_media_sender,
        ..MediaRequirements::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn start_command(
    id_generator: &dyn IdGenerator,
    ids: &MediaPortContractIds,
    media_session_id: MediaSessionId,
    media_binding_id: MediaBindingId,
    operation_id: OperationId,
    reservation: &crate::MediaReservation,
    payload: CommandPayload,
) -> MediaNodeCommand {
    MediaNodeCommand {
        request_id: id_generator.generate_correlation_id().to_string(),
        tenant_id: ids.tenant_id,
        media_session_id,
        media_binding_id,
        media_node_id: reservation.media_node_id,
        media_node_instance_epoch: reservation.media_node_instance_epoch,
        operation_id,
        owner_epoch: cheetah_signal_types::OwnerEpoch(1),
        source_node_id: id_generator.generate_node_id(),
        deadline: None,
        idempotency_key: id_generator.generate_message_id().to_string(),
        contract_version: reservation.contract_version,
        payload,
    }
}

fn assert_start_result(result: &MediaNodeCommandResult, label: &str) {
    assert!(
        matches!(
            result,
            MediaNodeCommandResult::Accepted | MediaNodeCommandResult::Completed
        ),
        "{label} start must be Accepted or Completed, got {result:?}"
    );
}

/// Runs the full [`MediaPort`] behavioral contract against `port`.
///
/// Panics on the first violated invariant so it can be used directly from a
/// `#[test]`/`#[tokio::test]` body. Callers provide the `id_generator` and
/// `clock` the port should use, allowing deterministic fake runs and real
/// system runs to share identical assertions.
pub async fn run_media_port_contract(
    port: &dyn MediaPort,
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
) {
    let ids = MediaPortContractIds::generate(id_generator);

    // --- Live reservation + start (viewer, no media sender). ---
    let live_session = id_generator.generate_media_session_id();
    let live_binding = id_generator.generate_media_binding_id();
    let live_op = id_generator.generate_operation_id();
    let live_reservation = port
        .reserve_live(
            ids.tenant_id,
            ids.device_id,
            ids.channel_id,
            live_session,
            live_binding,
            MediaPurpose::Live,
            &requirements("live", false),
            clock,
        )
        .await
        .expect("reserve_live must succeed");
    assert!(
        live_reservation.contract_version >= 1,
        "reservation must advertise a versioned contract"
    );

    let live_start = port
        .execute(
            start_command(
                id_generator,
                &ids,
                live_session,
                live_binding,
                live_op,
                &live_reservation,
                CommandPayload::StartLive {
                    media_session_id: live_session,
                    channel_id: ids.channel_id,
                    media_node_id: live_reservation.media_node_id,
                    purpose: MediaPurpose::Live,
                },
            ),
            clock,
        )
        .await
        .expect("execute(StartLive) must succeed at the transport level");
    assert_start_result(&live_start, "live");

    // The node must now be queryable and expose the started session.
    let _nodes = port
        .list_nodes(ids.tenant_id, clock)
        .await
        .expect("list_nodes must succeed");
    let sessions = port
        .list_sessions(
            ids.tenant_id,
            live_reservation.media_node_id,
            PageRequest::default(),
            clock,
        )
        .await
        .expect("list_sessions must succeed");
    assert!(
        sessions
            .items
            .iter()
            .any(|s| s.media_session_id == live_session),
        "started live session must be observable on its media node"
    );

    // --- Broadcast reservation + start (media sender required). ---
    let broadcast_session = id_generator.generate_media_session_id();
    let broadcast_binding = id_generator.generate_media_binding_id();
    let broadcast_op = id_generator.generate_operation_id();
    let broadcast_reservation = port
        .reserve_broadcast(
            ids.tenant_id,
            ids.device_id,
            ids.channel_id,
            broadcast_session,
            broadcast_binding,
            &requirements("broadcast", true),
            clock,
        )
        .await
        .expect("reserve_broadcast must succeed for a media-sender session");

    let broadcast_start = port
        .execute(
            start_command(
                id_generator,
                &ids,
                broadcast_session,
                broadcast_binding,
                broadcast_op,
                &broadcast_reservation,
                CommandPayload::StartBroadcast {
                    media_session_id: broadcast_session,
                    channel_id: ids.channel_id,
                    media_node_id: broadcast_reservation.media_node_id,
                },
            ),
            clock,
        )
        .await
        .expect("execute(StartBroadcast) must succeed at the transport level");
    assert_start_result(&broadcast_start, "broadcast");

    // --- Stop converges. ---
    let stop = port
        .execute(
            start_command(
                id_generator,
                &ids,
                live_session,
                live_binding,
                id_generator.generate_operation_id(),
                &live_reservation,
                CommandPayload::StopMediaSession {
                    media_session_id: live_session,
                },
            ),
            clock,
        )
        .await
        .expect("execute(StopMediaSession) must succeed at the transport level");
    assert!(
        matches!(
            stop,
            MediaNodeCommandResult::Completed | MediaNodeCommandResult::Accepted
        ),
        "stop must converge, got {stop:?}"
    );

    // --- Release is idempotent. ---
    port.release(ids.tenant_id, live_binding, clock)
        .await
        .expect("release must succeed");
    port.release(ids.tenant_id, live_binding, clock)
        .await
        .expect("releasing an already-released binding must be harmless");
    port.release(ids.tenant_id, broadcast_binding, clock)
        .await
        .expect("release must succeed");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator, InMemoryMediaPort};
    use std::sync::Arc;

    /// GB4-MED-008: the deterministic fake media node satisfies the shared
    /// [`MediaPort`] contract. This runs in CI with no network dependency.
    #[test]
    fn fake_media_node_satisfies_media_port_contract() {
        let id_generator = Arc::new(InMemoryIdGenerator::new());
        let clock = InMemoryClock::new();
        let port = InMemoryMediaPort::new(id_generator.clone());
        futures::executor::block_on(run_media_port_contract(
            &port,
            id_generator.as_ref(),
            &clock,
        ));
    }
}
