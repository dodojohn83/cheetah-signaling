# ASM-004: Node 与 ownership 装配

## Summary

- Added stable `NodeId` resolution in `apps/cheetah-signaling/src/assembly.rs`:
  - If `system.node_id` is configured it is used directly.
  - Otherwise the id is read from `<data_dir>/node_id`.
  - If missing, a UUIDv7 id is generated, persisted, and reused on later starts.
- Generated a fresh `NodeInstanceId` each process start for fencing.
- Replaced the ad-hoc `StorageOwnerResolver` with the production
  `CachingDeviceOwnerResolver` from `cheetah-cluster-ownership`, which adds a
  bounded, TTL-aware cache and respects `lease_until`.
- Added `cheetah-cluster-ownership` and `cheetah-cluster-registry` to the
  application dependencies so cluster lease/heartbeat services can be wired
  in follow-up commits.

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

- `NodeLeaseService` / `OwnerLeaseService` background workers require
  `Storage` to vend `Arc<tokio::sync::Mutex<dyn Repository>>` for shared
  async access; the concrete repository types already support this pattern in
  the cluster crate tests and will be wired in the next iteration.

Refs: ASM-004
