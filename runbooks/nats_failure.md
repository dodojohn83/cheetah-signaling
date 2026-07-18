# Runbook: NATS Failure

## Meaning
The node cannot publish or receive messages through the configured NATS cluster. This breaks command dispatch to owner nodes, outbox delivery and cross-node event fan-out.

## Possible Causes
- NATS server restart or rolling upgrade.
- Network partition between the node and NATS.
- Incorrect NATS credentials or permissions (subject deny list).
- TLS certificate mismatch or expiry.
- JetStream stream/storage resource exhaustion.

## Diagnostic Commands
```bash
# Check NATS connection and permissions
nats server check connection --server nats://<host>:4222
nats account info --server nats://<host>:4222

# Review message bus logs
grep -E "nats|bus|JetStream" /var/log/cheetah-signaling/*.log

# Inspect pending outbox
GET /api/v1/admin/db-status
POST /api/v1/admin/outbox-replay  # after recovery
```

## Mitigation
1. Verify `CHEETAH_MESSAGING__NATS_URL` and credentials.
2. If TLS issues, verify the server certificate and `tls_ca_ref` secret.
3. Ensure the NATS user has publish/subscribe rights for `cheetah.commands.*` and `cheetah.events.*`.
4. Restart the affected signaling node after NATS is reachable; pending outbox entries can be replayed via `POST /api/v1/admin/outbox-replay`.

## Recovery Confirmation
- Outbox pending count drops after replay.
- Cross-node command dispatch succeeds (watch `cheetah_message_nats` published/delivered metrics).
- `GET /health/ready` stays `200`.
