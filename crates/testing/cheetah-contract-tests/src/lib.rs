//! Message-envelope and media-port contract tests for GB4-TST-003.
//!
//! # Responsibility
//!
//! This crate holds the *message* and *media* contract suites that complement
//! the repository contracts in `cheetah-storage-tests` and the architecture
//! contracts in `cheetah-architecture-test`:
//!
//! - **message** (`tests/message.rs`) — command/event envelope encode/decode
//!   round-trips with metadata preservation, plus the in-process bus delivery
//!   semantics (FIFO command ordering, at-least-once redelivery, duplicate
//!   dedup by message id, and broadcast event fan-out).
//! - **media** (`tests/media_port.rs`) — the [`cheetah_domain::MediaPort`]
//!   contract exercised against the deterministic
//!   [`cheetah_domain::in_memory::InMemoryMediaPort`]: reserve/release,
//!   duplicate-reservation rejection, start/stop/control command results,
//!   tenant isolation and media-node instance-epoch stamping.
//!
//! # Boundaries
//!
//! - Tests are deterministic: IDs come from
//!   [`cheetah_domain::in_memory::InMemoryIdGenerator`] and time from
//!   [`cheetah_domain::in_memory::InMemoryClock`].
//! - No RTP/RTCP or media payloads are handled: media coverage is control-plane
//!   only (reservations and typed node commands).
//! - This crate contains only test code and exposes no production API.
