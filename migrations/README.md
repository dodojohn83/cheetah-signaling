# Database migrations

This directory contains SQLx embedded migrations for SQLite and PostgreSQL.

## Naming convention

```text
<version>__<phase>_<description>.sql
```

Examples:

- `0001__initial.sql` — legacy baseline DDL, treated as `baseline`.
- `0006__expand_add_status.sql` — add a new column/table.
- `0006__migrate_status_index.sql` — add indexes/constraints for the new path.
- `0006__backfill_status.sql` — populate the new column in batches.
- `0007__switch_use_status.sql` — flip reads/writes to the new schema.
- `0008__contract_drop_old_status.sql` — remove the old column/table.

Phases are used for zero-downtime rolling upgrades:

1. **Expand** — add new columns/tables; old code path still works.
2. **Migrate** — add indexes needed by the new code path.
3. **Backfill** — populate new columns batch by batch; resumable via `_cheetah_backfill_jobs`.
4. **Switch** — flip application logic to the new schema.
5. **Contract** — drop old columns/tables after all nodes have been upgraded.

## Lifecycle

- Startup (`cheetah-signaling` start or the storage migration API) applies
  `baseline`, `expand`, and `migrate` only. This keeps the schema compatible
  with both the previous and the next binary version.
- Backfills are run separately via `Migration::run_backfills` so long data
  migrations can be paused, resumed, and throttled.
- `switch` migrations are applied after all nodes are on the new version and
  backfills are complete.
- `contract` migrations are delayed until the version after the switch, so a
  rollback of the binary does not hit missing columns or tables.

## Backfill SQL

Backfill scripts must be idempotent and process a single batch each time they
are executed. The runner substitutes `/*BATCH_SIZE*/` with the configured batch
size. A typical pattern is:

```sql
UPDATE devices
SET migrated = TRUE
WHERE id IN (
    SELECT id
    FROM devices
    WHERE migrated IS NULL
    ORDER BY id
    LIMIT /*BATCH_SIZE*/
);
```

The runner loops until a batch reports zero rows affected.

## Backup and rollback

Before any migration:

- Take a logical backup (`pg_dump` for PostgreSQL, `.dump` or file copy for
  SQLite) and verify it restores cleanly.
- Test the migration in a non-production environment with a representative
  dataset.
- Review the `switch` and `contract` phases to confirm the previous binary
  version can still read the schema.

Rollback rules:

- Rolling back the binary before `switch` is safe if the startup phases are
  compatible.
- After `switch`, rollback may require re-applying the old code path or
  restoring from backup.
- Never roll back after `contract` without restoring a backup, because the old
  columns/tables no longer exist.
- Irreversible migrations (e.g. destructive column drops) are only run in the
  `contract` phase, which is delayed by at least one release window.

## Per-version notes

### 0001–0005 (baseline)

Initial schema for tenants, devices, channels, operations, media sessions,
media bindings, outbox events, processed messages, device owners, plugin
instances, and audit logs.

These files are treated as `baseline` migrations and are applied automatically
at startup.
