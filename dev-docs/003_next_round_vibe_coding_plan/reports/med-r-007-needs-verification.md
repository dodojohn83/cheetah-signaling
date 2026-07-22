# MED-R-007: Scheduler/registry reconciler

## Summary

- Added `MediaBindingState::NeedsVerification` to the `MediaBinding` lifecycle.
- Updated `MediaBindingState` transition table and state-machine tests to allow
  `Reserved -> NeedsVerification`, `Active -> NeedsVerification`, and
  `NeedsVerification -> Active` (via `verified()`).
- Extended `MediaService::reconcile` to classify inactive media nodes by their
  lease/health status. When a node lease is expired or the node is unhealthy,
  existing active bindings are marked `NeedsVerification` instead of immediately
  failing the session.
- Extended callback and command handlers to accept callbacks and stop/release
  commands on bindings in `NeedsVerification` state.
- Added `MediaPort::get_node` so the reconciler can look up a missing node's
  runtime state before deciding whether to verify, migrate, or skip.
- Implemented `SchedulerMediaPort::get_node` and `InMemoryMediaPort::get_node`.
- Wired a periodic media reconciliation worker in `apps/cheetah-signaling` that
  pages through all tenants and invokes `MediaService::reconcile` at a
  configurable interval (default 30s), providing a recovery path for missed gap
  reconciliation requests.

## Environment

- Host: devin-box (Linux-5.15.200-x86_64, 2 CPUs)
- Toolchain: Rust 1.96.1, Edition 2024, Cargo resolver 3

## Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib --bins --tests
cargo test --doc --workspace
cargo deny check
python3 scripts/audit_architecture.py
python3 scripts/verify_gb4_fixtures.py
```

## Results

- `cargo fmt --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test --workspace --lib --bins --tests`: PASS
- `cargo test --doc --workspace`: PASS
- `cargo deny check`: PASS (pre-existing duplicate/allow-list warnings only)
- `python3 scripts/audit_architecture.py`: PASS (0 production `todo!`/`unimplemented!`/`panic!`)
- `python3 scripts/verify_gb4_fixtures.py`: PASS

## Notes

- Orphan cleanup (media node reports a session that has no signaling binding) is
  still recorded as `orphans_detected` with a warning. Issuing a `StopMediaSession`
  command for an orphan requires a command/operation model that does not assume an
  existing `MediaBinding`; this is left for a follow-up change.
- `MED-R-006` (event consumer bounded subscription, resume cursor, dedup, gap
  reconciliation trigger) remains on branch `devin/med-r-006-gap-reconciler`
  (PR #235) and is not covered by this PR.

Refs: MED-R-007
