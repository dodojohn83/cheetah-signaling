# Cheetah Signaling Threat Model

This document describes the high-value threats against the Cheetah Signaling control plane and the controls that mitigate them. It is a living document: new outbound access, parser changes, authentication mechanisms, secret stores and plugin permissions must update this model and the corresponding tests.

## Scope

- In scope: northbound HTTP/gRPC API, internal gRPC, NATS messaging, protocol drivers (GB28181/ONVIF), device/media state machines, storage, plugins and secret handling.
- Out of scope: media RTP/RTCP/PS/TS payload plane. The control plane must not process, forward, store or decode media payload bytes.

## Trust Boundaries

```
External devices ─┬─> HTTP/gRPC API ─┐
                  │                  ├─> Application services ─> Storage
                  ├─> GB28181/ONVIF ─┤                  │
                  │                  │                  └─> NATS bus
                  └─> Webhooks <─────┘
```

- Northbound clients are authenticated (API key or JWT) and tenant-scoped.
- Protocol drivers authenticate devices using protocol-specific credentials (SIP Digest, ONVIF WS-Security/user token) and map them to internal `TenantId`/`DeviceId`.
- Internal gRPC uses mTLS with peer identity extracted from the client certificate.
- NATS traffic is expected on a segregated network; the bus enforces subject-level publish/subscribe permissions.
- Storage may be SQLite (edge) or PostgreSQL (cluster); the migration DSN is separate from the runtime DSN.

## Threats and Controls

| ID | Threat | Control | Verification |
|----|--------|---------|--------------|
| T1 | Malformed SIP/XML/SOAP/HTTP/Proto causes parser panic or unbounded resource use. | Sans-I/O parsers with incremental input, size/depth/count limits, no `unwrap`/`expect` on input; bounded buffers and channel depths. | Fuzz regression, malformed input unit tests, clippy `unwrap_used` denied. |
| T2 | Weak or missing device authentication lets an attacker register/control devices. | GB28181 SIP Digest, ONVIF WS-Security; credential references stored, not plaintext; rotation with dual-version transition. | Auth tests, secret scan, rotation tests. |
| T3 | Outbound HTTP requests reach internal services (SSRF). | URL allow-lists, scheme/port validation, internal IP blocklist, DNS-rebinding TOCTOU checks; no credential or raw body logging. | SSRF unit tests, reserved-range tests. |
| T4 | Secret leakage in logs, traces, error responses, or Debug output. | `SecretString` and zeroize on temp buffers; stable error codes in HTTP/gRPC; no raw protocol bodies in default logs; audit excludes secrets. | Secret leak tests, log scan fixtures. |
| T5 | Plugin or vendor SDK escapes isolation and accesses storage/media internals. | Out-of-process plugin gRPC bridge; in-process adapters only map to `MediaPort`/`CommandBus`; filesystem and network sandboxed by host. | Plugin contract tests, architecture tests. |
| T6 | Cross-tenant access or tenant ID spoofing. | `TenantId` newtype carried everywhere; repository queries always include tenant; RBAC scope checks in handlers. | Cross-tenant API tests, repository contract tests. |
| T7 | Message forgery or replay on NATS/command bus. | Message envelopes carry message IDs and idempotency keys; inbox deduplication; owner epoch fencing. | Duplicate-delivery tests, epoch tests. |
| T8 | Admin API abuse (drain, migrate, replay, diagnostics). | `system_admin` scope; audit events recorded; diagnostics sanitized, rate-limited and volume-capped. | Admin auth tests, audit tests. |
| T9 | Expired or mis-issued TLS/mTLS certificates. | Secret provider stores cert/key references; identity must match node ID; rolling-upgrade compatibility matrix. | Certificate validation tests. |
| T10 | Resource exhaustion (disk full, connection pool, timer wheel, queues). | Bounded caches, channels, batches, pages; timer wheel overflow logs and drops; disk-full runbook. | Admission controller and queue-full tests. |
| T11 | Observable high-cardinality identifiers become Prometheus labels. | Only status families and request totals are labels; device/session/request IDs go to trace/log fields, not metrics labels. | Metrics endpoint tests. |

## Data Flow Notes

- Authentication secret material never enters `Debug`/`Serialize` output; the `Authorization` header value and API key are not logged.
- Protocol bodies are logged only when `protocol_body_logging` is explicitly enabled, and then only after sanitization, within time/body limits, and with an audit record.
- W3C `traceparent`/`tracestate` propagate through HTTP headers, gRPC metadata and NATS message envelopes, but trace state does not contain secrets.
- High-cardinality identifiers (`device_id`, `session_id`, `request_id`) may appear in structured logs and traces; they must not be Prometheus label names or label values.

## Responsibilities

- New parsers, outbound clients, auth methods, secret stores or plugin permissions must update this table and add a matching test.
- Security-critical changes require an updated runbook entry if they change an operational response (e.g., new rate limit, new cert rotation flow).
