# GB4-SEC-001 — GB28181 Threat Model and Mitigation Mapping

## 1. Status and scope

- **Task:** `GB4-SEC-001`
- **Status:** Completed as a threat-model and verification-planning deliverable
- **Date:** 2026-07-21
- **System:** Cheetah Signaling GB28181 control plane
- **Primary report:** This document
- **Related specification:** [07 security, observability and operations](../07_security_observability_and_operations.md)

This report identifies the trust boundaries, abuse cases, required mitigations, and
evidence IDs for GB28181 signaling. It does not claim that the future implementation
tasks listed below are already complete. The signaling service must not receive, parse,
store, or forward RTP/RTCP or other media payloads; media endpoint decisions remain
behind `MediaPort`.

## 2. Assets and trust boundaries

### Assets

- Tenant, device, channel, platform, session, subscription, operation, and media
  binding identities.
- Device credentials, platform credentials, Digest server secret, SecretStore
  references, and mTLS identities.
- Protocol session ownership, owner epoch, session generation, media instance epoch,
  command idempotency state, and audit records.
- Catalog, RecordInfo, Alarm, MobilePosition, location, PTZ, and DeviceControl data.
- Resource budgets: listeners, connections, transactions, dialogs, aggregations,
  subscriptions, queues, timers, and bridge links.
- Logs, traces, fixtures, parser diagnostics, and error responses.

### Trust boundaries

1. Untrusted UDP/TCP SIP peers to the listener and framing/parser.
2. Authenticated device/platform identity to tenant and resource authorization.
3. SIP/XML/SDP protocol modules to application commands and persistent state.
4. Signaling control plane to `MediaPort` and media-node callbacks.
5. Local operator/configuration/SecretStore boundary.
6. Cluster nodes, NATS/DB consumers, and cascade peers across owner epochs.
7. Diagnostic exporters and stored fixtures to operators and external tooling.

## 3. Threat model

| ID | Threat and impact | Required mitigation | Test, fixture, or follow-up |
| --- | --- | --- | --- |
| `TM-01` | Unauthenticated `REGISTER` flood consumes parser, nonce, transaction, session, and database capacity; an unauthenticated `MESSAGE`, `INVITE`, or `SUBSCRIBE` flood can create dialogs, aggregations, media work, or subscriptions. | Apply pre-body admission and bounded framing; rate-limit by source, tenant, listener, method, and device; require Digest before state-changing work; cap connections, transactions, dialogs, subscriptions, and aggregations; reject or drop safely when saturated; never create a session or operation from an unauthenticated request. | `GB4-SIP-003`, `GB4-SIP-004`, `GB4-ARC-003`; `GB4-TST-002` access/overload transitions; `GB4-TST-004` register/command/subscription storm scenario; `GB4-SYS-005` security and overload report. |
| `TM-02` | Digest nonce replay or `nc` rollback repeats registration/control side effects; stale nonce handling can become an oracle or a password-failure amplifier. | Generate nonce from injected randomness and server secret with tenant/realm/TTL/context; track nonce use and monotonic `nc`/request context per session; reject replay before side effects; distinguish stale from bad credentials; bound credential backend work and do not log nonce material. | `GB4-SIP-004`; `GB4-ACC-002`; `GB4-TST-001` Digest malformed/replay corpus; `GB4-TST-002` replay transition table; existing `testdata/gb28181/sip/register.*` as the baseline fixture family. |
| `TM-03` | Digest qop or algorithm downgrade, including silent MD5 fallback, permits weaker authentication or policy bypass. | Prefer SHA-256; allow MD5 only through an explicit pinned compatibility profile; reject unsupported qop/algorithm combinations; fail production startup for challenge-optional or insecure policy; expose bounded auth outcome metrics without credentials. | `GB4-SIP-004`; `GB4-COMP-001`; `GB4-COMP-004`; `GB4-TST-001` algorithm/qop fixtures; `GB4-SYS-003` device interoperability evidence. |
| `TM-04` | Digest brute-force and credential enumeration exhaust CPU/SecretStore or reveal valid device/tenant identities. | Enforce per-source, per-tenant, per-device, and global auth budgets with exponential backoff/temporary blocking; use constant-shape failure responses; bound SecretStore calls and cache only safe policy data; audit security events without secrets. | `GB4-SIP-004`; `GB4-OPS-003`; `GB4-TST-004` brute-force/admission scenario; `GB4-SYS-005` sensitive-data and overload report. |
| `TM-05` | Realm/domain/DeviceID confusion maps a valid credential or body to another tenant, device, channel, or listener. | Resolve tenant from the trusted listener/domain/realm route; require To/From, request URI, authenticated identity, XML `DeviceID`, and persisted `ProtocolSession` mapping to agree; use typed tenant/device identities and tenant-scoped repositories; reject mismatch before any write or command. | `GB4-SIP-005`; `GB4-ACC-003`; `GB4-TST-002` tenant/body mismatch cases; `GB4-SYS-001` and `GB4-SYS-002` tenant isolation contract tests. |
| `TM-06` | Forged Via/Contact/source/rport causes response hijacking, endpoint migration, NAT pinning abuse, or a stale peer to take over a session. | Parse and validate Via/Contact/source/rport independently; treat observed source as transport evidence, not identity; update route only after authenticated renewal and current session generation; apply listener/network-zone policy to advertised addresses; fence owner takeover with epoch and revision. | `GB4-SIP-006`; `GB4-ACC-003`; `GB4-TST-001` endpoint/route fixtures; `GB4-TST-002` endpoint drift and owner takeover; `GB4-SYS-005` endpoint hijack report. |
| `TM-07` | SIP header CRLF/obs-fold/token injection, duplicate or ambiguous `Content-Length`, and oversized TCP frames desynchronize parser state, poison logs, or cause memory/CPU exhaustion. | Use incremental framing with one canonical `Content-Length`; reject duplicates/conflicts, obs-fold, invalid tokens, CRLF injection, excessive header count/value length, and oversized start line/body/frame; never concatenate untrusted wire values into logs or generated messages; close slow or invalid TCP peers. | `GB4-SIP-001`; `GB4-TST-001` malformed SIP corpus and arbitrary TCP slice property tests; `GB4-TST-003` driver/core contract; `GB4-SYS-005` parser safety evidence. |
| `TM-08` | XML DTD/XXE or external resource loading reads local/network resources; depth, node, text, item, extension, or decompression bombs exhaust parser resources. | Disable DTD, external entities, and external resource loading; enforce depth/node/text/item/extension/body and decompressed-size limits; use bounded incremental parsing and explicit charset/profile policy; reject malformed encoding and never include full XML in errors/logs. | `GB4-TST-001` DTD/XXE/depth/node/text corpus; `GB4-TST-003` parser contract; `GB4-SYS-005` no-resource-access and no-leak evidence. |
| `TM-09` | SDP address, port, payload type, attribute, or line injection redirects media negotiation, probes internal networks, or causes unauthorized media binding. | Treat SDP as untrusted control metadata; validate typed address/port/payload/attribute values and line/media counts; apply listener/network-zone policy and advertised-address rules; reject invalid/private/unauthorized endpoints; signaling never connects to SDP media endpoints directly—`MediaPort` performs endpoint authorization and binding. | `GB4-MED-003`; `GB4-COMP-003`; `GB4-TST-001` SDP injection/SSRF corpus; `GB4-TST-003` MediaPort fencing contract; `GB4-SYS-002` real media control-plane boundary test. |
| `TM-10` | Catalog and RecordInfo fragmentation, duplicate, reorder, huge counts, or slow streams create unbounded aggregation state or corrupt channels. | Key aggregations by tenant/device/command/SN/operation; cap active aggregations, fragments, items, bytes, and deadline; deduplicate and validate declared versus actual counts; return `Partial`/`Failed` rather than fabricate completeness; use bounded batches and transactional channel mapping. | `GB4-ACC-005`; `GB4-TST-001` huge/duplicate/reordered XML fixtures; `GB4-TST-002` aggregation transition table; `GB4-SYS-005` overload evidence. |
| `TM-11` | Alarm and location floods exhaust durable queues, cause silent loss, or create high-cardinality metrics and storage amplification. | Apply tenant/source/device quotas and priority-aware bounded inbox/outbox; deduplicate stable protocol events; coalesce only explicitly low-value presence/location updates; preserve critical alarms or dead-letter with reason; keep device/session IDs out of metric labels. | `GB4-EVT-002`; `GB4-OPS-001`; `GB4-OPS-003`; `GB4-TST-003` message contract; `GB4-SYS-005` queue saturation/cardinality report. |
| `TM-12` | Replayed, duplicated, delayed, or stale-owner PTZ/DeviceControl commands repeat dangerous actions or report false success. | Use typed commands with message ID, idempotency key, deadline, operation/step ID, tenant, session generation, and owner epoch; perform inbox dedupe and capability/RBAC checks before dispatch; reject old epoch before send and on response; continuous PTZ has server-side stop/deadline; uncertain execution is `UnknownOutcome`, never an automatic business retry. | `GB4-CMD-001..003`; `GB4-TST-002` command outcome/old epoch cases; PTZ and DeviceControl fixtures in `GB4-TST-001`; `GB4-SYS-005` replay/unknown-outcome report. |
| `TM-13` | Cascade directory leakage or external-ID collision exposes another tenant's devices/channels; excessive subscriptions exhaust dialogs/timers; bridge loops multiply work or media bindings. | Persist platform links and subscriptions with tenant, ACL, owner/link generation, expiry, capacity, and external-ID mapping; paginate and filter directories; validate hop/loop identity and enforce a unique control owner; cap subscriptions/refresh work and compensate bridge Saga steps; require explicit endpoint/network policy. | `GB4-CAS-003..006`; `GB4-OPS-003`; `GB4-TST-002` cascade/loop/ACL transitions; `GB4-TST-003` cascade contract; `GB4-SYS-004` cascade interoperability report. |
| `TM-14` | Forged, replayed, duplicated, or late media callbacks cause an old media node instance to advance a new binding, leak state, or close the wrong session. | Authenticate internal callbacks with TLS/mTLS and node identity; require media node instance epoch, media binding generation, tenant, session, operation/step, correlation, and owner epoch; verify before and after side effects; reject stale/duplicate callbacks idempotently; reconcile persisted state after restart. | `GB4-MED-006..008`; `GB4-TST-003` MediaPort callback/fencing contract; `GB4-SYS-002` real media node callback test; `GB4-SYS-006` node restart/old-instance chaos scenario. |
| `TM-15` | Logs, traces, fixtures, parser diagnostics, or error responses leak passwords, Authorization/WWW-Authenticate, nonce material, private keys, full SIP/XML/SDP, Contact/RTSP userinfo, SQL parameters, or sensitive location data. | Centralize redaction before emission; log bounded method/status/size/error position/header names/transaction hash/profile only; disable raw sampling by default and require time/volume/encryption/audit controls; make secret types non-debuggable/non-serializable; audit without raw protocol bodies and scrub fixtures. | `GB4-SEC-004`; `GB4-TST-001` redaction fixtures; `GB4-SYS-005` log/trace/error/audit snapshot scan; `GB4-OPS-005` diagnostic sampling runbook. |

## 4. Cross-cutting security invariants

1. Authentication, tenant/resource authorization, capability checks, and admission
   limits happen before durable or external side effects.
2. Every mutable protocol/application/media operation is tenant-scoped and carries
   the relevant session generation, revision, owner epoch, or media instance epoch.
3. Every parser and queue has explicit bounds. Saturation produces a defined reject,
   drop, coalesce, or dead-letter outcome and a bounded metric.
4. Duplicate input is safe: transaction responses, inbox receipts, command dispatch,
   event handling, and callbacks return the first known outcome without repeating
   side effects.
5. A SIP 2xx response means only the protocol-layer outcome where specified; it is
   never silently promoted to device-command or media-operation success.
6. Error, log, trace, audit, fixture, and report content is redacted by default.
7. The control plane does not accept or process media payloads; all media resource
   decisions cross the typed `MediaPort` boundary.

## 5. Evidence plan and completion boundary

`GB4-SEC-001` is complete when this threat-to-mitigation-to-evidence mapping is
reviewed and linked from the security/operations roadmap. The referenced implementation
and test IDs remain the owners of executable controls and evidence:

- SIP framing, routing, Digest, and endpoint behavior: `GB4-SIP-001..006`.
- Tenant/session/access and event handling: `GB4-ACC-001..005`, `GB4-EVT-001..002`.
- Typed command safety and outcomes: `GB4-CMD-001..003`.
- Media callback and binding fencing: `GB4-MED-006..008`.
- Cascade isolation, subscription capacity, ACL, and loop prevention:
  `GB4-CAS-003..006`.
- Parser, contract, simulator, security, and system evidence:
  `GB4-TST-001..004`, `GB4-SYS-002`, `GB4-SYS-004..005`.
- Redaction and operational diagnostics: `GB4-SEC-004`, `GB4-OPS-001`,
  `GB4-OPS-003`, and `GB4-OPS-005`.

No passwords, authorization values, nonce material, complete protocol bodies,
real addresses, or personal information are included in this report.
