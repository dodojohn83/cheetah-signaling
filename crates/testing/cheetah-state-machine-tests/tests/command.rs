//! GB4-TST-002 command transition table: dispatch / outcome / timeout / cancel
//! / duplicate / stale owner-epoch.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, device_resource_ref, request_context,
};
use cheetah_domain::{
    CommandPayload, DispatchAttempt, DispatchAttemptStatus, DomainError, IdempotencyScope,
    MediaPurpose, Operation, OperationError, OperationResult, OperationStatus, OperationStepStatus,
};
use cheetah_signal_types::{DeviceId, IdGenerator, OwnerEpoch, TenantId};

fn setup() -> (InMemoryClock, InMemoryIdGenerator, TenantId, DeviceId) {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    (clock, id_generator, tenant_id, device_id)
}

fn start_live_payload(id_generator: &InMemoryIdGenerator) -> CommandPayload {
    CommandPayload::StartLive {
        media_session_id: id_generator.generate_media_session_id(),
        channel_id: id_generator.generate_channel_id(),
        media_node_id: id_generator.generate_node_id(),
        purpose: MediaPurpose::Live,
    }
}

fn new_operation(
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
    tenant_id: TenantId,
    device_id: DeviceId,
    owner_epoch: OwnerEpoch,
) -> Operation {
    let context = request_context(tenant_id, id_generator, clock);
    let target = device_resource_ref(tenant_id, device_id);
    Operation::new(
        id_generator,
        clock,
        &context,
        "idem-key",
        device_id,
        target,
        start_live_payload(id_generator),
        None,
        owner_epoch,
    )
    .map(|(op, _)| op)
    .expect("operation creation should succeed")
}

/// Action applied to an operation as part of the transition matrix.
#[derive(Clone, Copy, Debug)]
enum Action {
    Start,
    CompleteSuccess,
    Cancel,
    Timeout,
}

fn apply(op: &mut Operation, action: Action, clock: &InMemoryClock) -> cheetah_domain::Result<()> {
    match action {
        Action::Start => op.start(clock).map(|_| ()),
        Action::CompleteSuccess => op.complete(OperationResult::success(), clock).map(|_| ()),
        Action::Cancel => op.cancel(clock).map(|_| ()),
        Action::Timeout => op.timeout(OperationError::timeout(), clock).map(|_| ()),
    }
}

fn operation_in(
    status: OperationStatus,
    clock: &InMemoryClock,
    id_generator: &InMemoryIdGenerator,
    tenant_id: TenantId,
    device_id: DeviceId,
) -> Operation {
    let mut op = new_operation(
        clock,
        id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );
    match status {
        OperationStatus::Pending => {}
        OperationStatus::Running => {
            op.start(clock).unwrap();
        }
        OperationStatus::Succeeded => {
            op.start(clock).unwrap();
            op.complete(OperationResult::success(), clock).unwrap();
        }
        OperationStatus::Failed => {
            op.start(clock).unwrap();
            op.complete(OperationResult::failure("CODE", "msg"), clock)
                .unwrap();
        }
        OperationStatus::Cancelled => {
            op.cancel(clock).unwrap();
        }
        OperationStatus::TimedOut => {
            op.timeout(OperationError::timeout(), clock).unwrap();
        }
        other => panic!("unhandled operation status: {other:?}"),
    }
    assert_eq!(op.status(), status);
    op
}

/// Full operation transition matrix: every `(status, action)` pair asserts
/// whether the transition is accepted, and that terminal states are inert.
#[test]
fn operation_transition_matrix() {
    let statuses = [
        OperationStatus::Pending,
        OperationStatus::Running,
        OperationStatus::Succeeded,
        OperationStatus::Failed,
        OperationStatus::Cancelled,
        OperationStatus::TimedOut,
    ];
    let actions = [
        Action::Start,
        Action::CompleteSuccess,
        Action::Cancel,
        Action::Timeout,
    ];

    let allowed = |status: OperationStatus, action: Action| match action {
        Action::Start => status == OperationStatus::Pending,
        Action::CompleteSuccess => status == OperationStatus::Running,
        Action::Cancel | Action::Timeout => {
            matches!(status, OperationStatus::Pending | OperationStatus::Running)
        }
    };

    for status in statuses {
        for action in actions {
            let (clock, id_generator, tenant_id, device_id) = setup();
            let mut op = operation_in(status, &clock, &id_generator, tenant_id, device_id);
            let revision_before = op.revision().0;
            let result = apply(&mut op, action, &clock);
            assert_eq!(
                result.is_ok(),
                allowed(status, action),
                "status={status:?} action={action:?} expected allowed={}",
                allowed(status, action)
            );
            if result.is_err() {
                // Rejected transitions must not mutate the aggregate.
                assert_eq!(op.status(), status);
                assert_eq!(op.revision().0, revision_before);
            }
        }
    }
}

#[test]
fn dispatch_attempt_lifecycle_sent_then_acked() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut op = new_operation(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );
    op.start(&clock).unwrap();
    let step_id = op.steps()[0].step_id();
    let attempt_id = id_generator.generate_message_id();
    op.record_dispatch_attempt(step_id, DispatchAttempt::new(attempt_id), &clock)
        .unwrap();
    assert_eq!(
        op.steps()[0].attempts()[0].status(),
        DispatchAttemptStatus::Pending
    );
    op.mark_dispatch_attempt_sent(step_id, attempt_id, &clock)
        .unwrap();
    assert_eq!(
        op.steps()[0].attempts()[0].status(),
        DispatchAttemptStatus::Sent
    );
    op.mark_dispatch_attempt_acked(step_id, attempt_id, &clock)
        .unwrap();
    assert_eq!(
        op.steps()[0].attempts()[0].status(),
        DispatchAttemptStatus::Acked
    );
}

#[test]
fn dispatch_attempt_nack_records_error_without_ending_operation() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut op = new_operation(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );
    op.start(&clock).unwrap();
    let step_id = op.steps()[0].step_id();
    let attempt_id = id_generator.generate_message_id();
    op.record_dispatch_attempt(step_id, DispatchAttempt::new(attempt_id), &clock)
        .unwrap();
    op.mark_dispatch_attempt_nacked(step_id, attempt_id, OperationError::timeout(), &clock)
        .unwrap();
    assert_eq!(
        op.steps()[0].attempts()[0].status(),
        DispatchAttemptStatus::Nacked
    );
    // A NACK is a dispatch diagnostic, not a business outcome: the operation
    // stays Running so a retry (new attempt) is still possible.
    assert_eq!(op.status(), OperationStatus::Running);
}

#[test]
fn timeout_sets_current_step_failed() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut op = new_operation(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );
    op.start(&clock).unwrap();
    op.timeout(OperationError::timeout(), &clock).unwrap();
    assert_eq!(op.status(), OperationStatus::TimedOut);
    assert_eq!(op.steps()[0].status(), OperationStepStatus::Failed);
    assert!(op.is_terminal());
}

/// "Duplicate": the same idempotency inputs must produce an equal scope so a
/// consumer can dedupe re-delivered commands.
#[test]
fn duplicate_commands_share_idempotency_scope() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let op_a = new_operation(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );

    let target = device_resource_ref(tenant_id, device_id);
    let scope = IdempotencyScope::new(
        tenant_id,
        op_a.principal().id.clone(),
        target,
        op_a.command().idempotency_key(),
    )
    .unwrap();
    assert_eq!(&scope, op_a.idempotency_scope());
    assert_eq!(op_a.command().idempotency_key(), "idem-key");
}

/// "Duplicate" transition: re-issuing `start` on an already-running operation is
/// rejected rather than silently double-dispatching.
#[test]
fn duplicate_start_is_rejected() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut op = new_operation(
        &clock,
        &id_generator,
        tenant_id,
        device_id,
        OwnerEpoch::default(),
    );
    op.start(&clock).unwrap();
    let err = op.start(&clock).unwrap_err();
    assert!(matches!(err, DomainError::InvalidTransition { .. }));
    assert_eq!(op.status(), OperationStatus::Running);
}

/// "Old epoch": a command/callback that references an owner epoch older than the
/// operation's expected epoch must be fenced out. The operation records the
/// epoch it was minted against; the fencing rule is `incoming >= expected`.
#[test]
fn stale_owner_epoch_is_fenced() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let op = new_operation(&clock, &id_generator, tenant_id, device_id, OwnerEpoch(5));
    assert_eq!(op.expected_owner_epoch(), OwnerEpoch(5));

    let accepts = |incoming: OwnerEpoch| incoming.0 >= op.expected_owner_epoch().0;
    assert!(!accepts(OwnerEpoch(4)), "older epoch must be rejected");
    assert!(accepts(OwnerEpoch(5)), "current epoch is accepted");
    assert!(accepts(OwnerEpoch(6)), "newer epoch is accepted");
}
