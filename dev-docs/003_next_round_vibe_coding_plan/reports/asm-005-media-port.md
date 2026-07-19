# ASM-005: MediaPort 装配

## Summary

- Removed `UnsupportedMediaPort` from `apps/cheetah-signaling/src/assembly.rs`.
- Wired `SchedulerMediaPort` backed by an `InMemoryMediaNodeRegistry` and
  `LeastLoadedScheduler`, using `MediaControlClient` for media node RPC.
- `MediaControlClient` is configured with `MediaClientConfig::default()` and
  the process `SecretStore` for mTLS client key resolution.
- Added `cheetah-media-scheduler` and `cheetah-media-client` dependencies to
  `apps/cheetah-signaling/Cargo.toml`.

## Environment

- Host: devin-box (Linux-5.15.200-x86_64, 2 CPUs)
- Toolchain: Rust 1.96.1, Edition 2024, Cargo resolver 3

## Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo deny check
buf format --diff --exit-code
buf lint
```

## Results

- `cargo fmt --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo nextest run --workspace`: PASS (680 passed, 6 skipped)
- `cargo deny check`: PASS (pre-existing duplicate/allow-list warnings only)
- `buf format --diff --exit-code`: PASS
- `buf lint`: PASS

## Notes

- `MediaClusterRegistry` gRPC server and media event consumer wiring remain for
  the dedicated media scheduler phase; the `MediaClusterRegistryService`
  implementation already exists in `cheetah-media-scheduler` and can be served
  once `cheetah-grpc-api` transport is added.
- Edge readiness policy for missing media nodes (`required`/`optional`) will be
  added together with the gRPC server and health/ready endpoint integration.

Refs: ASM-005
