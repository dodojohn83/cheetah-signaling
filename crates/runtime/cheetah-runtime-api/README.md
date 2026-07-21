# cheetah-runtime-api

Sans-I/O runtime API for the Cheetah Signaling device runtime.

This crate defines the portable interface between protocol core, protocol module
and the concrete runtime implementation. It contains no Tokio or I/O code.

## Public surface

- `DeviceActor` and `ActorContext`: portable actor API with timer scheduling and
  session registry access.
- `RuntimeMessage`: the fixed set of messages a shard can process.
- `Scheduler`, `AdmissionController`, `ShardRouter`, `SessionRegistry`: ports and
  value objects.
- `RuntimeMetrics` / `RuntimeMetricsSnapshot`: aggregate runtime health metrics
  (no per-device labels).
- `RuntimeConfig`, `RuntimeError`: shared configuration and error types.

## Allowed dependencies

- `cheetah-signal-types`, `cheetah-domain`
- `async-trait`, `thiserror`

## Forbidden dependencies

Tokio, Axum, Tonic, SQLx, async-nats, quick-xml, concrete protocol or media
clients.
