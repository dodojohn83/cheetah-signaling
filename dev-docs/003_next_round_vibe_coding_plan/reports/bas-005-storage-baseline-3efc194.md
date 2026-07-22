# BAS-005: Storage and Migration Baseline

Commit: `3efc194`
Date: 2026-07-22

## Summary

The repository provides a single shared contract suite for the SQLite and PostgreSQL storage adapters. Both backends run the same repository contract tests, cover tenant/revision/cursor/transaction/outbox/processed-message/owner/ownership scenarios, and include a corrupt-row test.

## Test command

```bash
cargo test -p cheetah-storage-tests --test sqlite
cargo test -p cheetah-storage-tests --test postgres
```

## Results

| Backend | Test | Outcome |
|---------|------|---------|
| SQLite  | `sqlite_contract_suite` | ok |
| PostgreSQL | `postgres_contract_suite` | ok |

## Contract coverage

The shared suite in `crates/testing/cheetah-storage-tests/src/contract.rs` invokes the following contract modules against each backend:

- `device`
- `channel`
- `operation`
- `media`
- `media_node`
- `protocol_session`
- `platform_link`
- `list`
- `outbox`
- `outbox_retry`
- `transaction`
- `processed_message`
- `owner`
- `ownership`
- `node`
- `webhook`
- `step`
- `unicode`

This covers:

- Tenant scoping (`Fixtures::tenant_id()` is used throughout).
- Revision optimistic concurrency (`Revision` checks in save/update operations).
- Cursor pagination (`list` contract).
- Transaction + outbox atomicity (`transaction` and `outbox` contracts).
- Inbox/processed-message deduplication (`processed_message` contract).
- Owner epoch and ownership fencing (`owner` and `ownership` contracts).
- Corrupt row handling (`postgres.rs` and `sqlite.rs` each contain `assert_negative_stored_revision_returns_internal`).

## Migration baseline

Migrations live in `migrations/` and are executed via `storage.migration().run().await` at the start of each contract suite. The migration table is keyed by logical version and is backend-agnostic. Released migrations are append-only and shared between SQLite and PostgreSQL.

## Test isolation

- SQLite tests use an in-memory/`:memory:` or temporary file database per run.
- PostgreSQL tests use `testcontainers-modules::postgres`, start a fresh container per test run, and close the storage handle afterwards.

## CI

Both suites are included in `cargo nextest run --workspace` / `cargo test --workspace`, as shown by the `nextest` CI job.