# cheetah-storage-postgres

PostgreSQL storage adapter for Cheetah Signaling.

Implements `cheetah-storage-api` using `sqlx` with separate read/write pools,
connection timeouts, `application_name` for traceability, and append-only migrations
shared with the SQLite adapter.

## Configuration

Use `PostgresStorage::new("postgres://...").await` to create a storage instance
with the same connection string for runtime and migrations.

For production deployments that separate the migration role from the runtime
role, use `PostgresStorage::new_with_roles(runtime_url, migration_url).await`.
The runtime pools are used for ordinary reads and writes, while the migration
pool is used exclusively for schema migrations.

The migrations can be applied via `storage.migration().run().await`.

## Dependencies

- `cheetah-domain` and `cheetah-storage-api` for the port layer.
- `sqlx` with `runtime-tokio`, `postgres`, `migrate`, `macros`, `time`, `uuid`, `json`.
- `tokio` for the runtime.
