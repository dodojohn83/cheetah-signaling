# cheetah-storage-api

Sans-I/O storage ports for Cheetah Signaling.

This crate defines the contract between the application layer and storage
adapters. It does not depend on SQLx, Tokio, PostgreSQL, or SQLite.

## Allowed dependencies

- `cheetah-domain` for the repository and unit-of-work ports.
- `cheetah-signal-types` for identity and timestamp types.
- `async-trait` and `thiserror` for trait ergonomics and errors.

## Forbidden dependencies

- `tokio`, `sqlx`, `rusqlite`, `tokio-postgres`, `libpq`, and any other
  concrete database or runtime crate.
