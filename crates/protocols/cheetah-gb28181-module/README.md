# cheetah-gb28181-module

GB28181 protocol module (layer 4) mapping SIP/XML messages to protocol-level
outputs. It depends on `cheetah-gb28181-core` for wire parsing and on
`cheetah-runtime-api` for the `DeviceActor` integration.

## Responsibility

- GB28181 domain and device configuration (`Gb28181Config`).
- Device ID validation according to GB/T 28181 rules.
- XML codec for registration, keepalive, catalog, device info/status, alarm,
  mobile position, device control and record info messages.
- Sans-I/O `Gb28181Module` state machine handling REGISTER, MESSAGE, INVITE,
  ACK, BYE and CANCEL as needed for device access.
- `Gb28181Actor` implementing `cheetah_runtime_api::DeviceActor` so the module
  can run inside the shard worker runtime.

## Allowed dependencies

- `cheetah-gb28181-core`, `cheetah-runtime-api`, `cheetah-signal-types`.
- `quick-xml`, `async-trait`, `thiserror`, `tracing`, `serde`, `bytes`,
  `secrecy`, `tokio` (only for `async-trait` runtime traits).

## Forbidden dependencies

- No direct SQLx, NATS, HTTP client, media client or `cheetah-signal-application`
  imports.
- No socket I/O (that belongs to `cheetah-gb28181-driver-tokio`).

## Public entry

`lib.rs` re-exports `config`, `device_id`, `xml`, `module` and `actor` modules.
