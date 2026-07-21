# cheetah-runtime-tokio

Tokio implementation of the Cheetah Signaling device runtime.

This crate provides fixed shard workers, a hierarchical timer wheel, bounded
MPSC mailboxes, and a `Runtime` entry point. It is the only runtime crate that
is allowed to depend on Tokio.

Each shard owns its device actors on a single task; a device key is routed to a
fixed shard by stable hash. Actors are created lazily on first message and
lazily unloaded when they stay idle past `RuntimeConfig::actor_idle_timeout_ms`
(authoritative state lives in repositories/Operations, so an unloaded actor is
transparently recreated on its next message). Mailboxes, the timer command
channel, the output channel, and the timer dispatch backlog are all bounded.

## Public surface

- `Runtime`: starts shard workers, a timer wheel, and returns an output stream.
- `Runtime::metrics()`: point-in-time `RuntimeMetricsSnapshot` of runtime
  health, backlog, and resource state (no high-cardinality device labels).
- `AdmissionController`: bounded admission to shard mailboxes.

## Allowed dependencies

- `cheetah-runtime-api`, `cheetah-signal-types` (transitive)
- `tokio`, `tracing`, `async-trait`, `thiserror`, `time`

## Forbidden dependencies

SQLx, async-nats, concrete protocol or media clients.
