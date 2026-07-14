# cheetah-runtime-tokio

Tokio implementation of the Cheetah Signaling device runtime.

This crate provides fixed shard workers, a hierarchical timer wheel, bounded
MPSC mailboxes, and a `Runtime` entry point. It is the only runtime crate that
is allowed to depend on Tokio.

## Public surface

- `Runtime`: starts shard workers, a timer wheel, and returns an output stream.
- `AdmissionController`: bounded admission to shard mailboxes.

## Allowed dependencies

- `cheetah-runtime-api`, `cheetah-signal-types` (transitive)
- `tokio`, `tracing`, `async-trait`, `thiserror`, `time`

## Forbidden dependencies

SQLx, async-nats, concrete protocol or media clients.
