# GB4-SYS-007: Capacity and Horizontal-Scaling Report

- Date: 2026-07-21
- Conclusion: **Blocked**
- Signaling base: `14545ab6b22371093e41fe549c0db4f9470d2f15`
- Scope: 100,000, 300,000, and 1,000,000 online devices, staged workload
  growth, and approximate horizontal scaling.

## Evidence boundary

No capacity run was executed. The requested base does not provide the required
fixed-shard simulator/scenario DSL and production cluster harness as a
reproducible capacity tool. The phase plan also requires the cluster path and
the real media/control contract before media-operation load can be represented
without confusing simulator evidence with real-media evidence.

Creating one task per device or timer, or reporting a synthetic successful
socket count, would violate the documented acceptance boundary. This report
therefore records a blocked gate and does not make a capacity claim.

## Required workload and measurements

| Stage | Required workload | Result |
| --- | --- | --- |
| 100,000 | 60-second keepalive, refresh skew, protocol/profile mix, 1%/5%/10% media operations | Not run |
| 300,000 | same workload with tenant/vendor skew and 10%/50% re-registration jitter | Not run |
| 1,000,000 | full staged load, failure recovery, fake media control nodes, horizontal scale comparison | Not run |

Each stage must record register/keepalive and operation throughput; P50/P95/P99
latencies; CPU/RSS/network/file descriptors; queue depth; timer lag; owner
distribution; DB/NATS load; reject/drop/dedupe rate; and recovery time. The
scenario must pin hardware, kernel, signaling commit, configuration, profile
ratios, seed, and transcript hash.

## Commands and environment

| Item | Result |
| --- | --- |
| `git rev-parse HEAD` | `14545ab6b22371093e41fe549c0db4f9470d2f15` |
| Host | Linux, x86_64, 2 vCPU |
| Fixed-shard capacity harness | Not available on this base |
| 100k/300k/1M load environment | Not provisioned |
| Cluster DB/NATS and media control nodes | Not provisioned |

No synthetic load was started. No signaling process received, parsed, stored,
or forwarded RTP/RTCP/PS/TS/ES payload.

## Blockers and exit criteria

1. Complete the fixed-shard simulator and deterministic fault scenario DSL
   (`GB4-TST-004`) without per-device resident tasks/timers.
2. Provide a pinned, observable cluster environment with scalable DB/NATS and
   fake media control nodes.
3. Run all three stages and a one-to-two-to-many gateway/shard comparison,
   recording the measurements and failure recovery evidence above.
4. Demonstrate no authoritative-state loss, unbounded queue/memory growth,
   timer lag runaway, or device-cardinality metric labels.

Until those conditions are met, `GB4-SYS-007` remains `Blocked`.
