# cheetah-contract-tests

Message-envelope and media-port contract tests for **GB4-TST-003**.

## Responsibility

This crate contains only test code. It complements the repository contracts in
`cheetah-storage-tests` and the architecture contracts in
`cheetah-architecture-test`:

| Area    | File                  | Coverage |
|---------|-----------------------|----------|
| message | `tests/message.rs`    | `encode_command`/`decode_command` and `encode_event`/`decode_event` round-trips with metadata (message/tenant/correlation/owner-epoch/idempotency/operation and aggregate sequence); in-process bus FIFO command ordering, at-least-once + idempotent (dedup by message id) consumption, event fan-out to all subscribers, and no-subscriber publish being a no-op |
| media   | `tests/media_port.rs` | `MediaPort` contract against `InMemoryMediaPort`: reserve/release, duplicate-reservation rejection, start/stop/control node-command results, device-command rejection, tenant isolation and deterministic media-node instance-epoch / `contract_version` stamping |

## Determinism

- IDs come from `cheetah_domain::in_memory::InMemoryIdGenerator`.
- Time comes from `cheetah_domain::in_memory::InMemoryClock`.
- Async tests use the current-thread `tokio` runtime; no wall-clock sleeps.

## Allowed dependencies

`cheetah-signal-types`, `cheetah-domain` (with `test-util`),
`cheetah-message-api`, `cheetah-message-local`, `tokio` — all as
`dev-dependencies`.

## Forbidden dependencies / boundaries

- No production/runtime dependencies (`[dependencies]` is empty); `publish =
  false` and no runtime API is exposed.
- No RTP/RTCP or media payloads: media coverage is control-plane only
  (reservations and typed node commands).
- Repository transaction/outbox/revision/tenant contracts remain in
  `cheetah-storage-tests`; architecture layer checks remain in
  `cheetah-architecture-test`.
