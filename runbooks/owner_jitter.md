# Runbook: Owner Jitter

## Meaning
Device ownership epochs are changing rapidly between nodes, causing command dispatch failures, duplicate work and media callback rejections.

## Possible Causes
- Split-brain due to lease renewal delays.
- Clock skew between cluster nodes.
- Network flapping causing repeated owner re-election.
- `OwnerEpoch` not propagated from old owner to new owner on takeover.
- Reconciler running too aggressively.

## Diagnostic Commands
```bash
# Look for owner epoch changes
grep -E "owner_epoch|owner changed|fencing" /var/log/cheetah-signaling/*.log

# Check ownership table in storage
SELECT tenant_id, device_id, owner_node_id, epoch, updated_at FROM ownership ORDER BY updated_at DESC LIMIT 100;

# Trigger ownership reconciliation
# (automated reconcile is pending; use the ownership table query above)
GET /api/v1/admin/db-status
```

## Mitigation
1. Synchronize clocks across all nodes (NTP/PTP).
2. Increase lease TTL and reduce re-election sensitivity if network is flaky.
3. Investigate the node that keeps losing ownership; restart if unstable.
4. Ensure takeover path atomically increments `OwnerEpoch` and fences old callbacks.

## Recovery Confirmation
- `owner_epoch` values are monotonically increasing per `(tenant, device)`.
- `fencing rejected` log lines stop.
- Command dispatch success rate returns to normal.
- No inconsistencies remain in the ownership table; automated reconciliation is pending.
