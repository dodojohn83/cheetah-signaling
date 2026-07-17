# Runbook: Media Node Failure

## Meaning
One or more media nodes are unreachable, unhealthy or have crashed. New media sessions cannot be scheduled on the failed node and existing sessions may be interrupted.

## Possible Causes
- Media node process crash or out-of-memory kill.
- Network partition between signaling and media control plane.
- Media node certificate/key mismatch for mTLS.
- Media node disk full for recordings or logs.
- Scheduling bug leaving orphan `MediaBinding`s.

## Diagnostic Commands
```bash
# Query media node health and orphan bindings
POST /api/v1/admin/reconcile

# Review media-related logs
grep -E "media_node|MediaBinding|reservation" /var/log/cheetah-signaling/*.log

# Check media node control port
curl -k https://<media-node>:<port>/health

# Verify mTLS certificate expiry
openssl s_client -connect <media-node>:<port> -showcerts < /dev/null
```

## Mitigation
1. If a node is down, the scheduler should route new sessions to healthy nodes once the failed node is marked `Offline`.
2. For orphan `MediaBinding`s, trigger reconciliation via `POST /api/v1/admin/reconcile`.
3. Restart or replace the failed media node; ensure it registers with a new `instance_epoch`.
4. If disk is full, archive or delete old recordings and restart.

## Recovery Confirmation
- `GET /api/v1/media-nodes` shows the failed node as `Offline` or replaced.
- New `MediaSession`s reach `Active` on a healthy node.
- Orphan `MediaBinding` count drops after reconciliation.
- Active streams resume or are re-established by clients.
