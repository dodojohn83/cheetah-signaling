# cheetah-cluster-ownership

Device owner lease, resolver, and routing for Cheetah Signaling clusters.

This crate implements the cluster-side ownership semantics required by Phase 09
and Phase 18:

- `OwnerLeaseService`: acquire, renew, batch-renew, and release device leases
  through the storage `OwnerRepository`.
- `CachingDeviceOwnerResolver`: a `DeviceOwnerResolver` implementation that
  caches the result of `OwnerRepository::get` with a short TTL and respects the
  `lease_until` deadline. The cache is bounded by `max_capacity` and evicts the
  least-recently-used entry when the limit is reached; expired entries are
  removed before LRU eviction.
- `DeviceAssignmentService`: assigns devices to cluster nodes using a stable
  hash, preserving existing owners when they remain alive and eligible. It
  filters by node health, zone, protocol contract support, and capacity, and
  enforces global and per-node assignment rate limits.

All time-dependent logic uses the injected `Clock` so tests can run with a
`FakeClock`.

Allowed dependencies:
- `cheetah-domain`
- `cheetah-signal-types`
- `cheetah-storage-api`
- `async-trait`
- `thiserror`
- `tokio` (sync, time)
- `tracing`

Forbidden dependencies:
- Direct socket/HTTP/TLS/network clients
- SQLx, PostgreSQL or SQLite drivers
- NATS, Kafka or message broker clients
- Axum, Tonic or transport frameworks
- Media client implementations
