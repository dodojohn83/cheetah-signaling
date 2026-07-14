# cheetah-storage-tests

Shared repository contract tests for the `cheetah-storage-sqlite` and `cheetah-storage-postgres` adapters.

## Purpose

This crate defines a single contract suite that runs against any storage implementation conforming to the `cheetah-storage-api` `Storage` trait. The SQLite tests run on a temporary local database; the PostgreSQL tests start a real Postgres container via [`testcontainers`](https://testcontainers.com/).

## Usage

```bash
cargo test -p cheetah-storage-tests
```

To run only the SQLite tests:

```bash
cargo test -p cheetah-storage-tests sqlite
```

To run only the PostgreSQL tests, make sure Docker is available:

```bash
cargo test -p cheetah-storage-tests postgres
```

## Public API

- `cheetah_storage_tests::contract::run_all` — runs the full contract suite.
- `cheetah_storage_tests::fixtures::Fixtures` — deterministic builders and test inputs.

## Contract coverage

- Device/Channel/Operation/MediaSession/MediaBinding CRUD
- Revision optimistic concurrency
- Tenant isolation
- Idempotency lookups
- Outbox append/pending/mark-published
- Aggregate and outbox in the same transaction
- Transaction rollback
- Device owner leases
- Operation step records
- Unicode and special-character persistence
