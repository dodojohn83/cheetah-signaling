# cheetah-media-control-simulator

A fake media control node used to test Cheetah Signaling's media command and
query paths. It implements the `MediaControl`, `MediaQuery`, `MediaEventStream`
and `MediaClusterRegistry` gRPC services from `cheetah.common.v1`.

## Responsibilities

- Accept media commands (`NegotiateRtp`, `StartRtp`, `StopRtp`, proxy,
  recording, snapshots, control payloads) and return synthetic responses.
- Stream `MediaEvent` events back through `MediaEventStream`.
- Register/heartbeat/drain/deregister as a media node through
  `MediaClusterRegistry`.
- Simulate configurable per-command latency and failure rate for resilience
  testing.
- Keep an in-memory session list served by `MediaQuery`.

## Allowed dependencies

- `cheetah-signal-contracts` for generated gRPC types.
- `tonic`, `tokio`, `tokio-stream`, `futures`, `prost-types` for gRPC runtime.
- `clap` for CLI, `tracing`/`tracing-subscriber` for logs.
- `rand` for deterministic random failure injection.
- `uuid` for instance identifiers.

## Forbidden dependencies

- No SQLx, NATS, real media engine, protocol driver, or cluster registry.
- No `SystemTime::now()` in state machine logic (only used to populate wire
  timestamps).

## Features

- `default` only. All simulation behavior is controlled through CLI flags.

## Public entry points

- `cargo run -p cheetah-media-control-simulator -- --bind 127.0.0.1:50051 --seed 42`
