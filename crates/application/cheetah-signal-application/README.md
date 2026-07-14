# cheetah-signal-application

Application services for Cheetah Signaling.

## Responsibilities

- Device lifecycle: register, update, online/offline, capabilities, channel catalog, retire.
- Operation lifecycle: submit, cancel, complete, timeout.
- Command dispatch: resolve owner epoch, send via command bus, update operation state.
- Media lifecycle: start live/playback/talk, stop live, control playback.
- Event publishing: publish pending outbox events.

## Allowed dependencies

- `cheetah-domain` for domain aggregates and ports.
- `cheetah-signal-types` for shared types, clocks, ids, `RequestContext`, `Event`, `ResourceRef`, etc.
- `serde` for DTOs.
- `tracing` for observability.

## Forbidden dependencies

- Tokio, Axum, Tonic, SQLx, async-nats, quick-xml, and protocol-specific crates.
  These must only appear in transport/adapter crates.

## Public entry

The crate root exposes the services and DTOs. In-memory ports for testing are in
`cheetah-domain` under the `test-util` feature.
