# cheetah-perf

Performance, durability and stress scenario tests for the Cheetah Signaling control plane.

## Responsibilities

- Provide repeatable `#[ignore]` integration scenarios for edge baseline, cluster scale and storm/backpressure testing.
- Collect coarse latency and throughput metrics without relying on production infrastructure or public endpoints.
- Serve as the automated harness that produces the performance report artifacts required by the release gate.

## Allowed dependencies

- `cheetah-*` workspace crates (domain, types, application, storage, messaging, cluster, media, scheduler).
- `tokio`, `tempfile`, `tracing` from the workspace.
- `testcontainers-modules` for PostgreSQL/NATS scenarios.

## Forbidden dependencies

- Media runtime, codec or stream manager crates.
- Tools that require public cloud accounts, real devices or unbounded external network access.
- Criterion-style benchmark frameworks that do not fit the system-scenario model.

## Public entry points

- `cargo test -p cheetah-perf -- --ignored` runs all scenarios.
- `cargo test -p cheetah-perf --test edge_baseline -- --ignored` runs the edge baseline only.
- `cargo test -p cheetah-perf --test cluster_scale -- --ignored` runs the cluster scale only.
- `cargo test -p cheetah-perf --test storm_backpressure -- --ignored` runs the storm/backpressure scenario only.

Each test prints a JSON-ish summary of the scenario and writes it to stderr. The output is intentionally stable so it can be diffed against a baseline report.

## Features

- `default`: none.
