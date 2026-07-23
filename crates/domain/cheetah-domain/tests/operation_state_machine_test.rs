//! Operation state machine and lifecycle tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, device_resource_ref, request_context,
};
use cheetah_domain::{
    CommandPayload, DomainError, IdempotencyScope, MediaPurpose, Operation, OperationError,
    OperationResult, OperationStatus,
};
use cheetah_signal_types::{
    Clock, Deadline, DeviceId, DurationMs, IdGenerator, OwnerEpoch, TenantId,
};

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
    tenant_id: cheetah_signal_types::TenantId,
    device_id: DeviceId,
) -> Operation {
    let context = request_context(tenant_id, id_generator, clock);
    let target = device_resource_ref(tenant_id, device_id);
    Operation::new(
        id_generator,
        clock,
        &context,
        "key-1",
        device_id,
        target,
        start_live_payload(id_generator),
        None,
        OwnerEpoch::default(),
    )
    .map(|(op, _)| op)
    .expect("operation creation should succeed")
}

#[test]
fn operation_starts_in_pending() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    assert_eq!(operation.status(), OperationStatus::Pending);
    assert!(!operation.is_terminal());
    assert_eq!(operation.revision().0, 0);
}

#[test]
fn operation_transitions_pending_to_running() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let event = operation.start(&clock).expect("start should succeed");
    assert_eq!(operation.status(), OperationStatus::Running);
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::OperationStateChanged { .. }
    ));
    assert_eq!(operation.revision().0, 1);
}

#[test]
fn operation_completes_successfully() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.start(&clock).unwrap();
    let event = operation
        .complete(OperationResult::success(), &clock)
        .expect("complete should succeed");
    assert_eq!(operation.status(), OperationStatus::Succeeded);
    assert!(operation.result().unwrap().is_success());
    assert!(matches!(
        event,
        cheetah_domain::DomainEvent::OperationStateChanged { .. }
    ));
    assert_eq!(operation.revision().0, 2);
}

#[test]
fn operation_completes_with_failure() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.start(&clock).unwrap();
    operation
        .complete(OperationResult::failure("CODE", "message"), &clock)
        .unwrap();
    assert_eq!(operation.status(), OperationStatus::Failed);
    assert!(!operation.result().unwrap().is_success());
    assert_eq!(operation.result().unwrap().error_code(), Some("CODE"));
}

#[test]
fn operation_cancels_from_pending() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.cancel(&clock).unwrap();
    assert_eq!(operation.status(), OperationStatus::Cancelled);
    assert!(operation.is_terminal());
}

#[test]
fn operation_cancels_from_running() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.start(&clock).unwrap();
    operation.cancel(&clock).unwrap();
    assert_eq!(operation.status(), OperationStatus::Cancelled);
}

#[test]
fn operation_times_out_from_pending() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation
        .timeout(cheetah_domain::OperationError::timeout(), &clock)
        .unwrap();
    assert_eq!(operation.status(), OperationStatus::TimedOut);
    assert!(operation.is_terminal());
}

#[test]
fn operation_expires_before_dispatch() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.expire(&clock).unwrap();
    assert_eq!(operation.status(), OperationStatus::TimedOut);
    assert!(operation.error().is_some());
}

#[test]
fn operation_cannot_start_from_terminal() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.cancel(&clock).unwrap();
    let err = operation.start(&clock).unwrap_err();
    assert!(matches!(err, DomainError::AlreadyTerminal { .. }));
}

#[test]
fn operation_cannot_complete_from_pending() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let err = operation
        .complete(OperationResult::success(), &clock)
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidTransition { .. }));
}

#[test]
fn operation_cannot_cancel_after_success() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.start(&clock).unwrap();
    operation
        .complete(OperationResult::success(), &clock)
        .unwrap();
    let err = operation.cancel(&clock).unwrap_err();
    assert!(matches!(err, DomainError::InvalidTransition { .. }));
}

#[test]
fn operation_new_fails_with_nil_device_id() {
    let (clock, id_generator, tenant_id, _device_id) = setup();
    let context = request_context(tenant_id, &id_generator, &clock);
    let target = device_resource_ref(tenant_id, DeviceId::from_uuid(uuid::Uuid::nil()));
    let result = Operation::new(
        &id_generator,
        &clock,
        &context,
        "key",
        DeviceId::from_uuid(uuid::Uuid::nil()),
        target,
        start_live_payload(&id_generator),
        None,
        OwnerEpoch::default(),
    );
    assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
}

#[test]
fn operation_revision_increments_on_transitions() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let initial = operation.revision().0;
    operation.start(&clock).unwrap();
    operation
        .complete(OperationResult::success(), &clock)
        .unwrap();
    assert_eq!(operation.revision().0, initial + 2);
}

#[test]
fn operation_timestamps_use_injected_clock() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let created_at = operation.created_at();
    clock.advance(DurationMs::from_millis(100));
    operation.start(&clock).unwrap();
    assert!(operation.updated_at() > created_at);
    assert_eq!(
        operation.updated_at().as_offset().unix_timestamp_nanos()
            - created_at.as_offset().unix_timestamp_nanos(),
        100_000_000
    );
}

#[test]
fn operation_deadline_elapses_after_clock_advance() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let context = request_context(tenant_id, &id_generator, &clock);
    let now = clock.now_wall();
    let deadline =
        Deadline::from_now(now, DurationMs::from_millis(500)).expect("deadline should be valid");
    let (operation, _) = Operation::new(
        &id_generator,
        &clock,
        &context,
        "key",
        device_id,
        device_resource_ref(tenant_id, device_id),
        start_live_payload(&id_generator),
        Some(deadline),
        OwnerEpoch::default(),
    )
    .unwrap();
    assert_eq!(operation.deadline(), Some(deadline));
    clock.advance(DurationMs::from_millis(1000));
    assert!(deadline.is_elapsed(clock.now_wall()));
}

#[test]
fn idempotency_scope_rejects_empty_key() {
    let (_clock, _id_generator, tenant_id, device_id) = setup();
    let target = device_resource_ref(tenant_id, device_id);
    let result = IdempotencyScope::new(tenant_id, "principal", target, "");
    assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
}

#[test]
fn idempotency_scope_equality_is_stable() {
    let (_clock, _id_generator, tenant_id, device_id) = setup();
    let target = device_resource_ref(tenant_id, device_id);
    let a = IdempotencyScope::new(tenant_id, "principal", target.clone(), "key").unwrap();
    let b = IdempotencyScope::new(tenant_id, "principal", target, "key").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.idempotency_key, b.idempotency_key);
}

#[test]
fn operation_idempotency_scope_matches_input() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    assert_eq!(operation.idempotency_scope().idempotency_key, "key-1");
    assert_eq!(operation.command().idempotency_key(), "key-1");
    assert_eq!(operation.command().operation_id(), operation.operation_id());
}

#[test]
fn operation_result_rejects_oversized_code_and_message() {
    let result = OperationResult::failure("x".repeat(129), "msg");
    assert!(matches!(
        result.validate(),
        Err(DomainError::InvalidArgument { .. })
    ));

    let result = OperationResult::failure("code", "x".repeat(2049));
    assert!(matches!(
        result.validate(),
        Err(DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn operation_error_rejects_oversized_code_and_message() {
    let error = OperationError::new("x".repeat(129), "msg");
    assert!(matches!(
        error.validate(),
        Err(DomainError::InvalidArgument { .. })
    ));

    let error = OperationError::new("code", "x".repeat(2049));
    assert!(matches!(
        error.validate(),
        Err(DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn operation_complete_rejects_oversized_failure_result() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    operation.start(&clock).unwrap();
    let result = OperationResult::failure("code", "x".repeat(2049));
    let err = operation.complete(result, &clock).unwrap_err();
    assert!(matches!(err, DomainError::InvalidArgument { .. }));
}

#[test]
fn operation_fail_rejects_oversized_code_and_message() {
    let (clock, id_generator, tenant_id, device_id) = setup();
    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let err = operation.fail("x".repeat(129), "msg", &clock).unwrap_err();
    assert!(matches!(err, DomainError::InvalidArgument { .. }));

    let mut operation = new_operation(&clock, &id_generator, tenant_id, device_id);
    let err = operation
        .fail("code", "x".repeat(2049), &clock)
        .unwrap_err();
    assert!(matches!(err, DomainError::InvalidArgument { .. }));
}
