# GB4-CMD-003: Separate inbox receipt, dispatch attempt and OperationStep outcome

## Summary

Replaced the conflated `CommandHandlerResult` (which forced `ProcessedMessageStatus::Completed` for all command dispatches) with three explicit concerns:

- `InboxReceipt` - `Accepted` / `Duplicate` / `Rejected` / `DeadLetter`
- `CommandDispatch` - `Queued` / `Sent` / `TransportFailed` / `TimedOut`
- `OperationStepOutcome` - `Succeeded` / `Failed` / `Unknown` / `Cancelled`

The inbox service now records `ProcessedMessageStatus::Accepted` when a command is accepted and dispatched but its business outcome is not yet known, instead of falsely marking it `Completed`. The `CommandDispatcher` now marks the first `DispatchAttempt` as `Sent` (not `Acked`) when the message is handed to the command bus, matching the real transport semantics.

## Changes

- `crates/application/cheetah-signal-application/src/inbox.rs`
  - Added `InboxReceipt`, `CommandDispatch`, `OperationStepOutcome`, and the new `CommandHandlerResult` with helper constructors.
  - `InboxService` maps the receipt to `ProcessedMessageStatus` and serializes dispatch/outcome into the result payload.
- `crates/domain/cheetah-domain/src/ports.rs`
  - Added `ProcessedMessageStatus::Accepted`.
- `crates/storage/cheetah-storage-postgres/src/repository.rs` / `crates/storage/cheetah-storage-sqlite/src/repository.rs`
  - Updated string mapping for `ProcessedMessageStatus` to include `accepted`.
- `crates/application/cheetah-signal-application/src/command_dispatcher.rs`
  - Changed `mark_dispatch_attempt_acked` to `mark_dispatch_attempt_sent` on successful `command_bus.send`.
- `apps/cheetah-signaling/src/workers.rs`
  - `OwnerCommandHandler` returns `Accepted` + `Sent` + `Unknown` for GB28181/plugin commands and `Rejected` for unsupported commands or missing infrastructure.
- `crates/application/cheetah-signal-application/tests/inbox_service_test.rs`
  - Updated `RecordingHandler` to use the new `CommandHandlerResult` API and assert `ProcessedMessageStatus::Accepted`.

## Verification

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test --workspace --lib --bins --tests             # pass
python3 scripts/audit_architecture.py                   # no new violations
```

## Remaining work

- Response/event correlation that resolves the `Unknown` `OperationStepOutcome` to `Succeeded`/`Failed`/`TimedOut` will be handled in subsequent command/event tasks (`GB4-CMD-*` / `GB4-EVT-*`).
