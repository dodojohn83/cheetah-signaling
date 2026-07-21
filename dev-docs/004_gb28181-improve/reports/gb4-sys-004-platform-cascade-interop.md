# GB4-SYS-004: Platform Cascade Interoperability Report

- Date: 2026-07-21
- Conclusion: **Blocked**
- Signaling base: `14545ab6b22371093e41fe549c0db4f9470d2f15`
- Scope: one upstream platform and one downstream platform, including registration,
  catalog sharing, subscriptions, bridge control, ACLs, tenant mapping, and
  owner migration.

## Evidence boundary

This report records why the platform-level acceptance run cannot yet be closed.
It is not a claim of interoperability. The required platform harness and
external platform fixtures were not available on the requested base.

The phase tracker still has the prerequisite cascade tasks
`GB4-CAS-001..006` and compatibility tasks `GB4-COMP-001..004` open. Those tasks
provide the persisted `GbPlatformLink`, production REGISTER/keepalive transport,
tenant-scoped external ID mapping, subscription lifecycle, bridge Saga, ACL and
loop protection, plus pinned compatibility profiles. Without them, a successful
simulator exchange would not be evidence for a production cascade.

## Required matrix

| Area | Required evidence | Result |
| --- | --- | --- |
| Upstream/downstream registration | REGISTER, Digest, keepalive, expiry, reconnect | Not run |
| Catalog | query/share/change Notify, pagination, external ID mapping | Not run |
| Events | Alarm and MobilePosition subscriptions, refresh and expiry | Not run |
| Media bridge | live/playback bridge, CANCEL/BYE/INFO, both-side compensation | Not run |
| Isolation | ACL, virtual directory, multi-tenant separation, loop/hop limits | Not run |
| Recovery | signaling/platform/media restart and owner migration | Not run |

The eventual run must record the upstream/downstream manufacturer, model,
firmware, platform commit/config, network topology, profile revision, sanitized
semantic transcript, unsupported capabilities, and cleanup result.

## Commands and environment

| Item | Result |
| --- | --- |
| `git rev-parse HEAD` | `14545ab6b22371093e41fe549c0db4f9470d2f15` |
| Host | Linux, x86_64, 2 vCPU |
| Database/NATS | No dedicated cascade acceptance environment provisioned |
| Reference peers | No pinned upstream/downstream platform fixture provisioned |
| Media peer | No fixed real-media contract environment provisioned |

No live platform, device, or media payload was sent. No signaling process
received, parsed, stored, or forwarded RTP/RTCP/PS/TS/ES payload.

## Blockers and exit criteria

1. Complete and test `GB4-CAS-001..006` and `GB4-COMP-001..004`.
2. Provide pinned upstream/downstream platform fixtures and a disposable
   network topology with explicit deadlines and cleanup.
3. Run the matrix above with production assembly, then attach sanitized
   transcripts and command exit codes.
4. Re-run after platform or media restart and demonstrate owner fencing,
   subscription cleanup, and bridge compensation.

Until those conditions are met, the task remains `Blocked`; this report is the
required evidence reference rather than a completion report.
