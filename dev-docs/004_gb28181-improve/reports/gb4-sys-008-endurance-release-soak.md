# GB4-SYS-008: Endurance and Release-Soak Report

- Date: 2026-07-21
- Conclusion: **Blocked**
- Signaling base: `14545ab6b22371093e41fe549c0db4f9470d2f15`
- Scope: 24-hour development endurance and 72-hour release-candidate soak
  with device, platform, media, dependency, and rolling-node churn.

## Evidence boundary

Neither soak run was started. A valid endurance result requires the completed
system, security, chaos, capacity, and release environments described by the
phase documents. The requested base does not provide a release candidate,
reproducible long-running workload, or the cluster/media/platform fixtures
needed to distinguish a control-plane leak from an unavailable dependency.

The absence of a run is intentional: a short local smoke test cannot establish
24-hour or 72-hour stability, and a simulator-only run cannot certify the
real-device/platform/media boundary.

## Required soak matrix

| Workload/fault | Development 24h | Release 72h |
| --- | --- | --- |
| Register refresh, device jitter, catalog/alarm/commands | Not run | Not run |
| 1%/5% media-session churn and media failure | Not run | Not run |
| Node rolling drain and signaling/media restart | Not run | Not run |
| DB/NATS/SecretStore short interruptions | Not run | Not run |
| Platform registration/subscription refresh | Not run | Not run |
| Compatibility-profile acceptance and strict rejection | Not run | Not run |

The eventual report must pin the release artifact, signaling/media commits,
scenario seed/configuration, topology, database/NATS versions, and all command
exit codes. It must sample RSS, object/timer/connection/transaction/dialog
counts, queue and consumer lag, outbox/inbox/dead-letter state, owner fencing,
expiry cleanup, and terminal Operation/MediaSession/MediaBinding state.

## Commands and environment

| Item | Result |
| --- | --- |
| `git rev-parse HEAD` | `14545ab6b22371093e41fe549c0db4f9470d2f15` |
| Host | Linux, x86_64, 2 vCPU |
| 24-hour development harness | Not provisioned |
| 72-hour release candidate | Not available |
| Cluster/platform/media soak topology | Not provisioned |

No endurance or soak traffic was started. No signaling process received,
parsed, stored, or forwarded RTP/RTCP/PS/TS/ES payload.

## Blockers and exit criteria

1. Close the prerequisite system, security, chaos, and capacity blockers and
   produce a pinned release candidate.
2. Provide deterministic long-running scenarios with bounded sampling,
   deadlines, cleanup, and restart/fault schedules.
3. Run both durations and attach time-series summaries and sanitized
   diagnostics, proving steady-state recovery and no monotonic resource growth.
4. Verify no stale-owner side effects, orphan media bindings, pending terminal
   steps, expired platform links, or secret/raw-body leakage.

Until those conditions are met, `GB4-SYS-008` remains `Blocked`.
