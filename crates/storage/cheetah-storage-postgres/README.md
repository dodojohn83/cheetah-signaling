# cheetah-storage-postgres

PostgreSQL storage adapter for Cheetah Signaling.

Implements `cheetah-storage-api` using `sqlx` with separate read/write pools,
connection timeouts, `application_name` for traceability, and append-only migrations
shared with the SQLite adapter.

## Configuration

Use `PostgresStorage::new("postgres://...").await` to create a storage instance.
The migrations can be applied via `storage.migration().run().await`.

## Migrations

Each migration is applied atomically inside a `BEGIN ... COMMIT` block using a
single connection and is protected by a `pg_advisory_lock` to serialize
multi-node startup. Because the migration runs inside a transaction,
statements such as `CREATE INDEX CONCURRENTLY` cannot be used in migration
files; use `CREATE INDEX IF NOT EXISTS` (or perform concurrent index builds
manually outside the migration runner).

## Dependencies

- `cheetah-domain` and `cheetah-storage-api` for the port layer.
- `sqlx` with `runtime-tokio`, `postgres`, `migrate`, `macros`, `time`, `uuid`, `json`.
- `tokio` for the runtime.
