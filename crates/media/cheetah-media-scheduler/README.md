# cheetah-media-scheduler

Media node registry, lease tracking, and scheduling for Cheetah Signaling.

## Responsibilities

- Maintain the set of registered media nodes, including capabilities, load,
  session counts, health, draining state and lease expiration.
- Provide a gRPC `MediaClusterRegistry` service so media nodes can register,
  heartbeat, drain and deregister.
- Score and select media nodes for a `MediaRequirements` request.
- Implement the domain `MediaPort` so the `MediaService` can reserve and release
  media node resources.

## Allowed dependencies

- `cheetah-domain`, `cheetah-signal-types`, `cheetah-signal-contracts`.
- `tonic`, `tokio`, `tracing` for the gRPC transport adapter.
- Standard library collections and synchronization primitives.

## Design notes

The registry stores node state in memory. Node state is rebuilt on restart
through re-registration and reconciled against persisted `MediaBinding` records
in later phases. Lease expiry is checked lazily during scheduling and by a
background worker.
