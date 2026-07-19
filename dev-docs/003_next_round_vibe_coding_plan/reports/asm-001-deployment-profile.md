# ASM-001: Explicit edge/cluster deployment profile

## Summary

- Added `DeploymentProfile` (`edge` / `cluster`) to `cheetah_signal_types::config`.
- Added optional `system.profile` to `SignalConfig`.
- When `system.profile` is omitted, `SignalConfig::validate` infers it from the backend settings:
  - `postgres` + `nats` + `cluster.enabled = true` => `cluster`
  - `sqlite` + `local` + `cluster.enabled = false` => `edge`
- `SignalConfig::validate` enforces the deployment profile constraints:
  - **edge**: `storage.backend` must be `sqlite`, `messaging.backend` must be `local`, `cluster.enabled` must be `false`.
  - **cluster**: `storage.backend` must be `postgres`, `messaging.backend` must be `nats`, `cluster.enabled` must be `true`.
- Updated `config.example.toml` to set `profile = "edge"` and document both modes and the inference fallback.
- Added config integration tests:
  - `cluster_profile_requires_postgres_nats_and_cluster_enabled`
  - `cluster_profile_is_inferred_when_omitted`
  - `edge_profile_rejects_postgres_backend`

## Environment

- Host: devin-box (Linux-5.15.200-x86_64, 2 CPUs)
- Toolchain: Rust 1.96.1, Edition 2024, Cargo resolver 3

## Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p cheetah-config -p cheetah-signal-types
```

## Results

- `cargo fmt --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test -p cheetah-config -p cheetah-signal-types`: PASS (29 passed)
- `cargo nextest run -p cheetah-config -p cheetah-signal-types`: PASS (43 passed)

## Notes

- `system.profile` is optional; when omitted it is inferred from storage/messaging/cluster settings, preserving backward compatibility for existing configs.
- Unsupported combinations fail at validation time instead of being silently downgraded at runtime.

Refs: ASM-001
