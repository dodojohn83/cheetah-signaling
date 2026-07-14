# cheetah-domain

Domain aggregates, value objects, ports and in-memory test helpers for Cheetah Signaling.

## Scope

- Device, Channel, Operation, Command, MediaSession and MediaBinding aggregates.
- Explicit state machines and idempotent lifecycle methods.
- Repository, command bus, media port, message bus and unit-of-work ports.
- In-memory fake implementations for unit tests.

## Dependency rules

- Allowed: `cheetah-signal-types`, `async-trait`, `thiserror`, `serde`, `time`, `uuid`.
- Forbidden: Tokio, Axum, Tonic, SQLx, async-nats, quick-xml or concrete protocol crates.

## Public entry

Use `cheetah_domain::{Device, Channel, Operation, Command, MediaSession, MediaBinding, DomainEvent, DomainError}`
and `cheetah_domain::ports::*` for repository/ports.
