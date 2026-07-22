//! Deterministic state-machine transition-table tests for GB4-TST-002.
//!
//! # Responsibility
//!
//! This crate holds *table-driven* transition tests for the control-plane state
//! machines described in
//! `dev-docs/004_gb28181-improve/08_testing_interop_performance_and_release.md`:
//!
//! - **access** — [`cheetah_domain::ProtocolSession`] register / refresh /
//!   keepalive / offline / owner assignment plus the GB28181 ingress
//!   authorization rules.
//! - **command** — [`cheetah_domain::Operation`] dispatch / outcome / timeout /
//!   cancel / duplicate / stale owner-epoch fencing.
//! - **catalog** — the GB28181 `Catalog` parser and the fragment / duplicate /
//!   reorder / missing / partial / crash / revision-conflict assembly contract.
//! - **media** — [`cheetah_domain::MediaSession`] and
//!   [`cheetah_domain::MediaBinding`] saga steps, late `200`, `CANCEL`/`BYE`,
//!   early media and stale media-node-instance fencing.
//! - **cascade** — the GB28181 upstream cascade register / backoff / deregister
//!   transitions exercised through the public `process` API.
//!
//! The tests present *full valid/invalid transition matrices* that complement
//! the per-scenario tests living inside `cheetah-domain` and
//! `cheetah-gb28181-module`; they are intentionally not a copy of those.
//!
//! # Boundaries
//!
//! - Tests are deterministic: they use [`cheetah_domain::in_memory::InMemoryClock`],
//!   [`cheetah_domain::in_memory::InMemoryIdGenerator`] and in-memory fixtures.
//! - No real devices, sockets, network access, timers or media payloads are
//!   used. Cascade transitions are driven purely through synthesized SIP
//!   messages and logical `now` values.
//! - This crate contains **only test code**; it exposes no production API.
//!
//! The actual tests live under `tests/` so they compile as integration tests
//! against the crates under test.
