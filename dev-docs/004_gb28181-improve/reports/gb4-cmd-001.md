# GB4-CMD-001: Typed query, PTZ, preset and DeviceControl payloads

## Summary

This task adds typed domain, REST and Protocol Buffer payloads for the four
GB28181 command classes called out in `04_device_access_commands_and_events.md`:

- PTZ movement (`Ptz` already existed; kept and exposed)
- PTZ preset actions (`Preset`)
- Queries (`Catalog`, `DeviceInfo`, `DeviceStatus`, `RecordInfo`, `PresetQuery`,
  `ConfigDownload`)
- Device controls (`Guard`, `AlarmReset`, `Record`, `TeleBoot`, `IFrame`,
  `DeviceConfig`)

## Domain types

New types in `crates/domain/cheetah-domain/src/command.rs`:

- `QueryCommand` / `QueryKind`
- `PresetCommand`
- `DeviceControlCommand` / `DeviceControlKind`

`CommandPayload` is extended with `Query`, `Preset` and `DeviceControl` variants.
`PresetAction` (defined in `channel.rs`) now derives `Hash` so it can be used
inside `PresetCommand`.

## REST endpoints

`crates/api/cheetah-http-api/src/handlers/commands.rs` adds:

- `POST /api/v1/devices/{id}/commands/ptz`
- `POST /api/v1/devices/{id}/commands/preset`
- `POST /api/v1/devices/{id}/commands/query`
- `POST /api/v1/devices/{id}/commands/device-control`

Each endpoint requires the `operator` scope and an `Idempotency-Key` header,
builds a `SubmitOperationRequest` with the corresponding `CommandPayload`, and
returns `202 Accepted` with a `Location` header pointing at the created
operation.

Optional query parameters:

- `deadline` (RFC 3339) overrides the default 30 second deadline
- `owner_epoch` (u64) overrides the default expected owner epoch of `1`

## Protocol Buffers

`proto/cheetah/control/v1/control.proto` `ChannelCommand` now carries a typed
`detail` oneof:

- `PtzCommand` + `PtzDirection` enum
- `PresetCommand` + `PresetAction` enum
- `QueryCommand` + `QueryKind` enum
- `DeviceControlCommand` + `DeviceControlKind` enum

The existing `command_type` and `payload` fields are retained for backward
compatibility. `crates/messaging/cheetah-message-api/src/mapper.rs` initializes
`detail` to `None` when encoding the legacy JSON envelope.

## Validation

```text
cargo fmt --all -- --check                               # pass
cargo clippy --workspace --all-targets -- -D warnings  # pass
cargo test --workspace                                   # pass (except pre-existing cheetah-message-nats doctest)
python3 scripts/audit_architecture.py                    # no new violations
```

`buf format` and `buf lint` could not be run locally because `buf` is not
installed in this environment; they will be exercised by CI.

## Remaining work

- `GB4-CMD-002`: route these commands through the GB protocol module to client
  transactions instead of the generic operation dispatch path.
- `GB4-CMD-003`: separate `Inbox` receipt, `DispatchAttempt` and
  `OperationStep` outcome semantics, including `UnknownOutcome`.
