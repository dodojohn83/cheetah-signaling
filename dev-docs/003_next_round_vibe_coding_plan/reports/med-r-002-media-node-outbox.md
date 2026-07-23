# MED-R-002: MediaNode repository outbox events

## Summary

- Added `DomainEvent::MediaNodeUpdated { node: MediaNode }` to `cheetah-domain`.
- Added `ResourceKind::MediaNode` and `ResourceId::MediaNode(NodeId)` to
  `cheetah-signal-types` so media node events can be stored in the outbox with
  the same `ResourceRef` envelope used by other aggregates.
- Extended `MediaNodeRepository` trait so `register`, `heartbeat`,
  `set_draining` and `deregister` accept a `Vec<Event<DomainEvent>>` and append
  them to `outbox_events` inside the same transaction that persists the node.
- Implemented `append_outbox_events` helpers in both
  `PostgresMediaNodeRepository` and `SqliteMediaNodeRepository`. The helpers
  patch the event `aggregate_sequence` and the embedded `MediaNode` revision
  to the value actually written to the database, then insert the full envelope
  into `outbox_events`.
- Updated `PersistentMediaNodeRegistry` to carry an `IdGenerator` and the local
  signal `node_id`, and to build a `MediaNodeUpdated` `Event` for every
  `register`, `heartbeat`, `drain` and `deregister` operation before passing it
  to the repository.
- `append_outbox_events` now overwrites the entire `MediaNodeUpdated` payload
  with the persisted row read back from the database, so the outbox event always
  matches the committed state and cannot diverge from the registry-computed
  snapshot.
- Updated `apps/cheetah-signaling/src/assembly.rs` to inject the
  `id_generator` and `node_id` into `PersistentMediaNodeRegistry::new`.
- Expanded the shared SQLite/PostgreSQL storage contract tests in
  `crates/testing/cheetah-storage-tests/src/contract/media_node.rs` with
  `node_updated_event` and `assert_media_node_outbox_event`, verifying that each
  mutating operation produces a correctly-sequenced outbox event.
- Added `RESOURCE_KIND_MEDIA_NODE` to `proto/cheetah/common/v1/common.proto` and
  mapped `ResourceId::MediaNode`/`ResourceKind::MediaNode` in
  `crates/messaging/cheetah-message-api/src/mapper.rs` so the proto
  `EventEnvelope.aggregate` field is populated correctly for published media-node
  events.
- Recomputed `MediaNode::health` in `PersistentMediaNodeRegistry::heartbeat` before
  building the `MediaNodeUpdated` outbox event, so the emitted notification
  reflects the latest load/session_count.

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
- `cargo deny check`: PASS (license exception warnings only)
- `python3 scripts/audit_architecture.py`: 0 new production `todo!`/`unimplemented!`/`panic!`.
  The script exits with the same 5 pre-existing dependency/layer violations seen on `main`
  (`cheetah-media-scheduler -> cheetah-media-client`, `cheetah-onvif-driver-tokio -> cheetah-onvif-module`,
  `cheetah-cluster-registry -> tokio`, `cheetah-signal-contracts -> tonic`/`tonic-prost`).
  No new violations were introduced by this change.
- `python3 scripts/verify_gb4_fixtures.py`: PASS

## Notes

- `MediaNodeRepository` mutating methods now require callers to supply outbox
  events. `PersistentMediaNodeRegistry` builds them; the in-memory scheduler
  tests and `MediaNodeRegistry` trait methods are unchanged because the event
  construction is a persistence concern.
- The contract tests mark each asserted media-node outbox event as `published`
  so that the generic outbox contract tests (`outbox.rs`) are not polluted by
  unrelated pending events.

Refs: MED-R-002
