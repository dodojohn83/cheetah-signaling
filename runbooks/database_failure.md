# Runbook: Database Failure

## Meaning
The signaling node cannot read from or write to its configured storage backend (SQLite/PostgreSQL). This affects registration, command dispatch, media session state and audit durability.

## Possible Causes
- Network partition between the node and the PostgreSQL server.
- PostgreSQL primary failover or credentials expired.
- SQLite file on a full or read-only filesystem.
- Storage migration not applied (`ready` probe fails).
- Connection pool exhaustion from a traffic spike.

## Diagnostic Commands
```bash
# Check migration status
POST /api/v1/admin/db-status

# Review storage-related logs
grep -E "storage|sqlx|migration" /var/log/cheetah-signaling/*.log

# Test connectivity (PostgreSQL)
pg_isready -h <host> -p <port> -U <user>

# Check disk space
df -h /var/lib/cheetah-signaling
```

## Mitigation
1. If migration is behind, run `POST /api/v1/admin/db-migrate`.
2. If the pool is exhausted, scale replicas or reduce `storage.max_connections` contention.
3. For PostgreSQL failover, update the runtime DSN via `CHEETAH_STORAGE__RUNTIME_URL` and restart.
4. For SQLite, move the database to a volume with free space and ensure write permissions.

## Recovery Confirmation
- `GET /health/ready` returns `200` with `{"status":"ready"}`.
- `POST /api/v1/admin/db-status` reports `status: current`.
- Operations and device writes succeed without storage errors in logs.
