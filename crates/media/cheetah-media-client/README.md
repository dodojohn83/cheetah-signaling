# cheetah-media-client

gRPC `MediaControl` client for Cheetah Signaling.

## Responsibilities

- Maintain a per-media-node connection pool keyed by endpoint.
- Execute media commands with request ID, session ID, deadline, idempotency key,
  contract version and tenant routing.
- Retry only clearly retryable gRPC status codes.
- Enforce a per-node concurrency limit and a simple circuit breaker.
- Translate transport failures into stable error types.

## Allowed dependencies

- `cheetah-signal-contracts`, `cheetah-signal-types`.
- `tonic`, `tokio`, `tracing` for the gRPC transport adapter.
- Standard library synchronization primitives.
