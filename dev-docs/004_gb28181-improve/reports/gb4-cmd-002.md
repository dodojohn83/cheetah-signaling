# GB4-CMD-002: Route application commands to GB28181 driver client transactions

## Summary

This task replaces the `cheetah/gb28181` plugin placeholder path with a dedicated
command bus that routes domain `Command`s directly into the GB28181 driver's
outbound SIP `MESSAGE` client transaction machinery.

The end-to-end flow is:

1. `OwnerCommandHandler` recognizes GB28181 command payloads (`Query`, `Ptz`,
   `Preset`, `DeviceControl`).
2. It resolves the device and optional channel external ids through the
   `UnitOfWork` (channel falls back to device id when no `external_id` metadata
   is present).
3. It sends a `Gb28181Command` into the driver-bound `mpsc` command bus.
4. The driver consumes the command, calls `GbAccessMachine::process_command`,
   starts a UDP client transaction, and sends the initial `MESSAGE` request.

## Core changes

### `crates/protocols/cheetah-gb28181-core/src/access.rs`

- Added `CommandInput` associated type to `GbAccessMachine`.
- Added `process_command` to `GbAccessMachine`.
- Extended `AccessOutput` with `SendMessage { target: SocketAddr, message: SipMessage }`.

### `crates/protocols/cheetah-gb28181-driver-tokio`

- `DriverConfig` gained `command_channel_capacity` (default `1024`) with a builder.
- `Gb28181UdpDriver` creates a bounded `mpsc` channel in `bind` and exposes
  `command_bus()` returning the sender.
- `run_with_cancellation` takes the `command_rx`, spawns a command consumer task,
  and uses `Shared::handle_command` to produce `DriverAction::Send` outputs for
  each outbound `MESSAGE`. Initial requests are sent on the first UDP socket.
- `Shared::handle_command` acquires the access lock, calls `process_command`,
  then starts a client transaction via `TransactionManager::start_client_transaction`
  for each `SendMessage` output.

### `crates/protocols/cheetah-gb28181-module`

- `Gb28181Command` wrapper carries a domain `Command`, device external id and
  optional channel external id.
- `Gb28181Access::process_command` is delegated to `access/outbound.rs` to keep
  `access.rs` under the 800-line guidance.
- `outbound::process_command` maps `CommandPayload::Query/Ptz/Preset/DeviceControl`
  to typed XML builders and produces a SIP `MESSAGE` request with
  `Content-Type: Application/MANSCDP+xml`.
- `xml/query.rs` added `QueryRequest` with `from_command` and `encode_xml` for
  `Catalog`, `DeviceInfo`, `DeviceStatus`, `RecordInfo`, `PresetQuery` and
  `ConfigDownload`.
- `xml/device_control.rs` extended `DeviceControlKind` with `Guard`, `AlarmReset`,
  `Record`, `TeleBoot`, `IFrame` and `DeviceConfig`, and implemented their XML
  encoding.

### `apps/cheetah-signaling`

- Added `Gb28181CommandBus` trait and `DriverCommandBus` implementation wrapping
  the driver's bounded `mpsc` sender.
- `OwnerCommandHandler` now takes an optional `Arc<dyn Gb28181CommandBus>` and
  dispatches GB28181 commands through it instead of `plugin_host`.
- `assembly.rs` creates the GB28181 driver before spawning the inbox worker and
  injects the first driver's command sender into `OwnerCommandHandler`.
- Non-GB commands and GB commands when no listener is configured still fall back
  to `unknown_outcome` without forging success.

## Tests

- Added `from_command_copies_kind_and_times` and `preset_query_xml` in
  `xml/query.rs`.
- Added XML encoding tests for `Guard`, `AlarmReset`, `Record`, `TeleBoot`,
  `IFrame` and `DeviceConfig` in `xml/device_control.rs`.
- Existing module, driver, application and storage tests continue to pass.

## Verification

```text
cargo fmt --all -- --check                              # pass
cargo clippy --workspace --all-targets -- -D warnings  # pass
cargo test --workspace --lib --bins --tests            # pass (pre-existing cheetah-message-nats doctest ignored as instructed)
python3 scripts/audit_architecture.py                   # no new violations; pre-existing warnings unchanged
```

`audit_architecture.py` reports the same pre-existing layer/dependency warnings
that are unrelated to this change:

- `cheetah-media-scheduler` -> `cheetah-media-client`
- `cheetah-onvif-driver-tokio` -> `cheetah-onvif-module`
- `cheetah-cluster-registry` -> `tokio`
- `cheetah-signal-contracts` -> `tonic` / `tonic-prost`

## Remaining work

- `GB4-CMD-003`: separate `Inbox` receipt, `DispatchAttempt` and `OperationStep`
  outcome semantics, including `UnknownOutcome`.
