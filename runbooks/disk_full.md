# Runbook: Disk Full

## Meaning
The node has run out of disk space, causing writes to fail, migrations to abort and possible process crashes.

## Possible Causes
- Unbounded log growth (structured logs without rotation).
- SQLite WAL files not checkpointed.
- Core dumps or temporary files accumulating.
- Large diagnostic packages saved to the data directory.

## Diagnostic Commands
```bash
# Check disk usage by mount
df -h

# Find the largest directories
ncdu /var/lib/cheetah-signaling /var/log/cheetah-signaling

# Check SQLite WAL size
ls -lh /var/lib/cheetah-signaling/*.sqlite*

# Check log rotation status
journalctl --vacuum-time=7d  # or equivalent logrotate state
```

## Mitigation
1. Free space by rotating logs, removing old core dumps and deleting stale diagnostic packages.
2. For SQLite, run `PRAGMA wal_checkpoint(TRUNCATE);` after backing up.
3. Move recordings or archives to cold storage.
4. Increase volume size if growth is expected.

## Recovery Confirmation
- `df` shows at least 20% free space on the data and log volumes.
- `GET /health/ready` returns `200`.
- New writes (device registration, outbox, audit) succeed without disk errors.
- Log rotation and retention policies are active.
