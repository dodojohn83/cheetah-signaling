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
  - Added `MultiListenerCommandBus` and resolved each command to the listener that the device registered on, so commands are not silently delivered to the wrong driver in multi-listener deployments.
  - Loads the active `ProtocolSession` to obtain the `listener_id` and passes it in `Gb28181Command`.
  - `resolve_gb_command` now propagates transient storage errors as retryable `SignalError`s, so the inbox can `nak` and redeliver instead of permanently rejecting a valid command during a database hiccup.
- `apps/cheetah-signaling/src/assembly.rs`
  - Collects all listener command senders and builds a `MultiListenerCommandBus` instead of only keeping the first sender.
- `crates/protocols/cheetah-gb28181-module/src/command.rs`
  - Added `listener_id` to `Gb28181Command`.
- `apps/cheetah-signaling/src/gb_event_sink.rs`
  - Persists `external_id` metadata for each catalog channel so channel-scoped GB28181 commands can address the correct SIP target.
  - Restored the `force ||` condition in `ensure_online` so re-registration can refresh the online state of an already-online device.
- `crates/protocols/cheetah-gb28181-core/src/sip/parser.rs`
  - Fixed a TCP stream-mode deadlock in the `HeaderNormalization` blank-line handling: a header terminator (`\r\n\r\n`) with no trailing bytes, or a body without a trailing CRLF, is now treated as the end of headers instead of waiting indefinitely for more data.
- `crates/application/cheetah-signal-application/tests/inbox_service_test.rs`
  - Updated `RecordingHandler` to use the new `CommandHandlerResult` API and assert `ProcessedMessageStatus::Accepted`.

## Verification

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings   # pass
cargo test --workspace --lib --bins --tests             # pass
python3 scripts/audit_architecture.py                   # no new violations
```

## Devin Review fixes

- `handle_media_session_event` now persists each `MediaSession` transition separately via `save_and_append_media_session_transition`, keeping the repository's `Revision` optimistic-concurrency check valid when the `Start`/`Stop` paths advance the session through multiple states in one go.
- `MultiListenerCommandBus` uses `try_send` so the inbox DB transaction is not held while waiting for a bounded driver channel.
- `resolve_gb_command` propagates transient storage errors as retryable `SignalError` instead of swallowing them as terminal `Rejected`.
- `ensure_online` restores the `force ||` condition so re-registration refreshes already-online devices.
- TCP `HeaderNormalization` blank-line handling no longer stalls on body-less `REGISTER` messages in stream mode.

## Remaining work

- Response/event correlation that resolves the `Unknown` `OperationStepOutcome` to `Succeeded`/`Failed`/`TimedOut` will be handled in subsequent command/event tasks (`GB4-CMD-*` / `GB4-EVT-*`).
