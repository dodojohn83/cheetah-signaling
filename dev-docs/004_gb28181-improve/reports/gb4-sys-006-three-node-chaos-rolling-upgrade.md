# GB4-SYS-006: Three-Node Chaos and Rolling-Upgrade Report

- Date: 2026-07-21
- Conclusion: **Blocked**
- Signaling base: `14545ab6b22371093e41fe549c0db4f9470d2f15`
- Scope: three signaling nodes, PostgreSQL, NATS JetStream, media-node
  restarts, ownership fencing, dependency faults, and rolling upgrade.

## Evidence boundary

The three-node chaos run was not executed. The current base contains the
device-test revision fix requested for this work, but the required cluster
environment and full GB/media recovery path are not available for a valid
system result. In particular, the phase plan still requires the cluster
PostgreSQL/NATS and real-media vertical path (`GB4-SYS-002` and the
`UP-MEDIA-P0`/`GB4-MED` dependencies) before this report can exercise the
production lifecycle.

## Required fault matrix

| Fault | Required assertion | Result |
| --- | --- | --- |
| Owner crash/lease expiry/old-owner return | takeover is bounded; stale epoch has no side effect | Not run |
| PostgreSQL delay/short outage/pool exhaustion | bounded work, recoverable outbox/repository state | Not run |
| NATS disconnect/ack loss/redelivery/lag | inbox dedupe and explainable replay/DLQ | Not run |
| Media restart/instance epoch change | old callback cannot advance a new binding | Not run |
| SecretStore timeout | no false success; readiness/retry behavior is bounded | Not run |
| Device/platform disconnect or register storm | admission and reconciliation converge | Not run |
| Rolling drain/upgrade | old/new versions coexist and drain without orphan state | Not run |

The eventual run must record node versions, schema/migration state, NATS
consumer configuration, fault schedule and seed, detection/takeover/recovery
times, operation/session/binding convergence, queue bounds, and sanitized
artifacts.

## Commands and environment

| Item | Result |
| --- | --- |
| `git rev-parse HEAD` | `14545ab6b22371093e41fe549c0db4f9470d2f15` |
| Host | Linux, x86_64, 2 vCPU |
| Three-node topology | Not provisioned |
| PostgreSQL/NATS fault harness | Not provisioned |
| Real media node | Not provisioned |

No chaos traffic was sent. No signaling process received, parsed, stored, or
forwarded RTP/RTCP/PS/TS/ES payload.

## Blockers and exit criteria

1. Provide a disposable three-node PostgreSQL/NATS/media topology with pinned
   commits and migration compatibility.
2. Complete the cluster/real-media workflow dependencies and expose deterministic
   fault injection with deadlines and cleanup.
3. Execute every fault above, including rolling drain and mixed-version
   operation, then attach sanitized timelines and command exit codes.
4. Prove bounded detection/takeover, old-epoch fencing, no unbounded backlog,
   and eventual convergence of all authoritative aggregates.

Until those conditions are met, `GB4-SYS-006` remains `Blocked`.
