# Runbook: Device Registration Storm

## Meaning
A large number of devices attempt to register or keep-alive simultaneously, causing CPU/memory saturation, owner lease contention or storage write spikes.

## Possible Causes
- Fleet reboot after a power/network outage.
- Misconfigured device retry storm without exponential backoff.
- DDoS against the public signaling endpoint.
- Certificate rotation causing mass reconnects.

## Diagnostic Commands
```bash
# Rate by source IP and protocol
grep -E "register|keepalive|REGISTER" /var/log/cheetah-signaling/*.log \
  | awk '{print $client_ip, $protocol}' | sort | uniq -c | sort -rn | head

# Check rate-limiter rejects
grep "RATE_LIMITED" /var/log/cheetah-signaling/*.log | tail -50

# Monitor active device actors and CPU
POST /metrics  # inspect cheetah_http_requests_total, cheetah_http_responses_4xx_total
```

## Mitigation
1. Confirm rate limits are enforced per `(source, tenant, protocol, node)`; tighten if needed.
2. Enable input shape limits (`SEC-003`) to reject oversized registration payloads early.
3. Scale signaling nodes horizontally; device ownership will be rebalanced across nodes.
4. Work with device vendors to add jitter/backoff to registration retries.

## Recovery Confirmation
- 4xx rate-limited responses stabilize at a low level.
- Device registration queue length (outbox/inbox backlog) trends to zero.
- CPU and memory usage return to baseline.
- `GET /health/ready` remains `200`.
