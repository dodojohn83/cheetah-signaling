# cheetah-storage-sqlite

SQLite storage adapter for Cheetah Signaling.

Implements `cheetah-storage-api` using `sqlx` with a single-writer pool
(`max_connections=1`), a separate read pool, WAL mode, foreign keys, and
append-only migrations shared with the PostgreSQL adapter.

## Configuration

Use `SqliteStorage::new(path).await` to create a storage instance. The
migrations can be applied via `storage.migration().run().await`.

## Dependencies

- `cheetah-domain` and `cheetah-storage-api` for the port layer.
- `sqlx` with `runtime-tokio`, `sqlite`, `migrate`, `macros`, `time`, `uuid`, `json`.
- `tokio` for the runtime.
