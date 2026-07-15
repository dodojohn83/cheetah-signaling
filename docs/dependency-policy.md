# Dependency and Toolchain Policy

This document records the initial frozen versions and the upgrade cadence for the Cheetah signaling workspace.

## Frozen initial versions

| Component | Pinned version | Notes |
|-----------|----------------|-------|
| Rust | `1.96.1` | Toolchain pinned by `rust-toolchain.toml`. |
| Tokio | `1.52.3` | Declared in root `Cargo.toml`; first used by protocol drivers and runtime. |
| Axum | `0.8.9` | HTTP transport adapter. |
| Tonic | `0.14.6` | Internal gRPC (media node, plugin host). |
| Prost | `0.14.4` | Protobuf codegen. |
| SQLx | `0.9.0` | Storage adapter (PostgreSQL + SQLite). |
| async-nats | `0.49.1` | Cluster bus and JetStream/KV. |
| quick-xml | `0.41.0` | ONVIF/SIP XML parsing. |
| rustls | `0.23.41` | TLS for HTTP/gRPC/NATS. |

All external versions are declared in the root `Cargo.toml` `[workspace.dependencies]` table. Individual crates must use `{ workspace = true }` and are not allowed to drift.

## Upgrade cadence

- **Patch releases**: reviewed monthly and applied if no advisory or contract failure.
- **Minor releases**: reviewed quarterly; each requires full `cargo nextest run --workspace`, proto breaking check, storage migration round-trip and aarch64 cross-check.
- **Major / toolchain**: reviewed every six months; requires interop, chaos and capacity regression before merge.
- Any version change must update this file, `rust-toolchain.toml`/`Cargo.toml`, CI and the baseline records in `dev-docs/002_vibe_coding_plan`.

## Deviation process

If a pinned version cannot be resolved or causes a conflict, the implementer must:

1. Record the minimal reproduction (`cargo update` output or CI log).
2. Propose an alternative version with identical major/minor compatibility.
3. Update this policy and the plan documents via an ADR before changing `Cargo.toml`.
