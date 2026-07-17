# cheetah-migration-tool

Standalone import tool for migrating tenants, devices, channels and secret
references from an old system into Cheetah Signaling.

## Responsibilities

- Reads source records from CSV or JSON.
- Maps old-system identities to deterministic UUIDs so re-runs are idempotent.
- Imports `Device` and `Channel` aggregates through the `Storage` port.
- Supports dry-run, cutover filtering and batch checkpoint commits.
- Refuses to migrate plaintext credentials; instead emits an action list for
the operator to re-enter secrets through the secret provider.

## Allowed dependencies

- `cheetah-config`, `cheetah-domain`, `cheetah-signal-types`, `cheetah-storage-api`,
  `cheetah-storage-postgres`, `cheetah-storage-sqlite`.
- `clap`, `csv`, `serde`, `serde_json`, `thiserror`, `tokio`, `tracing`, `tracing-subscriber`, `uuid`.

## Usage

```bash
# Dry-run against a SQLite target
cheetah-migration-tool --config config.toml --source old_devices.csv --dry-run

# Import a cutover list of devices
cheetah-migration-tool --config config.toml --source old_devices.csv \
  --cutover cutover.txt --checkpoint-every 50
```

The tool does not merge PRs automatically; after dry-run review, run again
without `--dry-run` to write to the target database.
