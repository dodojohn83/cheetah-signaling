# cheetah-gb28181-module

GB28181 protocol module: maps SIP/GB XML wire messages to domain events and
responses. This crate is Sans-I/O and does not perform network or database I/O;
its outputs are returned to a driver or application layer for execution.

## Responsibilities

- Tenant/realm/DeviceId parsing and validation.
- Digest authentication for REGISTER and other protected requests.
- Register/unregister, keepalive, catalog, device info/status, alarm, mobile
  position, device control and record-info workflows.
- Manufacturer/version compatibility profile selection.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP/Digest/XML primitives.
- Standard Rust crates, `secrecy`, `thiserror`, `tracing`.

## Forbidden dependencies

- Tokio, Axum, Tonic, SQLx, async-nats, quick-xml, or any concrete network,
  database, media or message broker client.

## Features

No optional features.

## Public entry

`lib.rs` re-exports:

- `AccessInput`, `AccessOutput`, `Gb28181Access` from `access`.
- `Gb28181DomainConfig`, `AuthPolicy`, `CharsetPolicy` from `config`.
- `AccessError` from `error`.
- `Gb28181Event`, `DevicePresence` from `events`.
- `CredentialProvider` from `ports`.
- `DeviceId`, `DomainId` from `types`.
