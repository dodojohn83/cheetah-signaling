# cheetah-cluster-registry

Cluster node lease, registry, and assignment support for Cheetah Signaling.

This crate provides the `NodeLeaseService` for process registration, heartbeat,
and draining. It depends on the `NodeRepository` port in `cheetah-storage-api`
and the `Clock`/`IdGenerator` ports from `cheetah-signal-types`.

Allowed dependencies:
- `cheetah-domain`
- `cheetah-signal-types`
- `cheetah-storage-api`
- `async-trait`
- `thiserror`
- `tokio` (sync only)
- `tracing`

Forbidden dependencies:
- Direct socket/HTTP/TLS/network clients
- SQLx, PostgreSQL or SQLite drivers
- NATS, Kafka or message broker clients
- Axum, Tonic or transport frameworks
- Media client implementations
