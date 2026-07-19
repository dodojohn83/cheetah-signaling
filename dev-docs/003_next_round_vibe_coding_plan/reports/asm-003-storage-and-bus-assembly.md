# ASM-003: Storage and message bus assembly

## Summary

- Wired storage backend selection in `apps/cheetah-signaling/src/assembly.rs`:
  - `sqlite` branch creates `SqliteStorage` and runs migrations.
  - `postgres` branch resolves `postgres_url_ref` / `postgres_url` and creates
    `PostgresStorage`.
- Wired messaging backend selection:
  - `local` branch creates `InProcessMessageBus`.
  - `nats` branch resolves `nats_url_ref` / `nats_url`, validates `tls://` or `wss://`
    scheme, and connects `NatsBus` with a 5s connect timeout and 30s operation timeout.
- Added `EventBusPublisher` adapter so a `RawEventBus` can be used as the domain
  `EventPublisher` expected by `OutboxRelay`.
- Generated a stable `NodeId` from config when set, otherwise a transient UUIDv7
  node id is generated for NATS cluster identity (with a logged warning).
- Updated `main.rs` to `Box::pin` the `assembly::start` future to avoid large-future
  lint.

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
- `cargo nextest run --workspace`: PASS (677 passed, 6 skipped)
- `cargo deny check`: PASS (pre-existing duplicate/allow-list warnings only)
- `buf format --diff --exit-code`: PASS
- `buf lint`: PASS

## Notes

- `NatsBus::connect` requires TLS for cluster deployments; assembly rejects plain
  `nats://` to enforce internal transport security.
- The `owner_resolver` passed to `NatsBus` is currently the storage-backed
  `StorageOwnerResolver`; ownership/lease service integration remains for ASM-004.

Refs: ASM-003
