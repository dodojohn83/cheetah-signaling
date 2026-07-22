# cheetah-state-machine-tests

Deterministic, table-driven state-machine transition tests for **GB4-TST-002**.

## Responsibility

This crate contains only test code. It exercises the control-plane state
machines through their public APIs and asserts full valid/invalid transition
matrices plus the scenario framings called out in
`dev-docs/004_gb28181-improve/08_testing_interop_performance_and_release.md`:

| Area     | File               | Coverage |
|----------|--------------------|----------|
| access   | `tests/access.rs`  | register / refresh / keepalive / unregister / owner assignment; ingress endpoint-update authorization matrix |
| command  | `tests/command.rs` | operation dispatch / outcome / timeout / cancel; full status matrix; duplicate (idempotency scope + repeated start); stale owner-epoch fencing |
| catalog  | `tests/catalog.rs` | fragment / duplicate / reorder / missing / partial / crash via the stateless `parse_catalog` parser and documented consumer contract |
| media    | `tests/media.rs`   | `MediaSession` and `MediaBinding` saga steps, early media, late `200`, `CANCEL`/`BYE`, full transition matrices, stale media-node-instance fencing |
| cascade  | `tests/cascade.rs` | register / duplicate / backoff / deregister / internal-upstream ACL driven through `Gb28181Cascade::process` |

## Determinism

- Time comes from `cheetah_domain::in_memory::InMemoryClock`.
- IDs come from `cheetah_domain::in_memory::InMemoryIdGenerator`.
- No real devices, sockets, network access, timers or media payloads are used.
  Cascade transitions are driven by synthesized SIP messages and logical `now`
  values.

## Allowed dependencies

`cheetah-signal-types`, `cheetah-domain` (with `test-util`),
`cheetah-gb28181-core`, `cheetah-gb28181-module`, `uuid`, `secrecy` — all as
`dev-dependencies`.

## Forbidden dependencies / boundaries

- No production/runtime dependencies (`[dependencies]` is empty); the crate is
  `publish = false` and exposes no runtime API.
- No async runtime, transport, database or media-plane crates.
- Tests must not reach into private module internals; cascade coverage uses the
  public `process` API only.

## Notes

- Catalog `revision-conflict` and the repository/message/media contracts are
  covered by `cheetah-storage-tests` and `cheetah-contract-tests` (GB4-TST-003).
- GB28181 subscription/bridge/loop transitions require in-crate fixtures and are
  covered by `cheetah-gb28181-module/src/cascade/tests`; this crate pins the
  public register/backoff/deregister/ACL contract.
