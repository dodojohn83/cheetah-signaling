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
- Explicit, auditable `CompatibilityProfile` schema (standard version, digest,
  charset, endpoint, catalog and SDP/MediaStatus overrides) with validation,
  capability negotiation, fixed-priority selection and revision pinning.
- `Gb28181Access`, an implementation of `cheetah_gb28181_core::GbAccessMachine`.

## Allowed dependencies

- `cheetah-gb28181-core` for Sans-I/O SIP/Digest primitives and the `GbAccessMachine` contract.
- `quick-xml` for GB28181 XML (MANSCDP / MANSRTSP) parsing and encoding.
- Standard Rust crates, `secrecy`, `thiserror`, `tracing`.

## Forbidden dependencies

- Tokio, Axum, Tonic, SQLx, async-nats, or any concrete network, database, media
  or message broker client.
- No `cheetah-plugin-sdk` or plugin-host types.

## Features

No optional features.

## Public entry

`lib.rs` re-exports:

- `Gb28181Access` from `access`.
- `AccessInput`, `AccessOutput`, `GbAccessMachine` from `cheetah-gb28181-core`.
- `Gb28181DomainConfig`, `AuthPolicy`, `CharsetPolicy` from `config`.
- `CompatibilityProfile`, `CompatibilityProfileConfig`, `CompatibilityRegistry`,
  `CompatibilityCapability`, `DeviceDescriptor`, `PinnedProfile` and the override
  value types from `compat`.
- `AccessError` from `error`.
- `Gb28181Event`, `DevicePresence` from `events`.
- `CredentialProvider` from `ports`.
- `DeviceId`, `DomainId` from `types`.
