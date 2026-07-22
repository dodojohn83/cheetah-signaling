//! Deterministic, fixed-shard GB28181 signalling simulator.
//!
//! This crate provides a reproducible, discrete-event simulator for GB28181
//! signalling.  A run is driven entirely by a [`scenario::Scenario`] loaded from
//! TOML plus its seed, and produces a [`report::RunReport`] that binds the seed,
//! scenario, message/fault counts, semantic outcomes, resource usage and a
//! transcript hash.
//!
//! # Design
//!
//! - A fixed number of shard workers manage many lazy device states; there is
//!   no per-device Tokio task or timer.
//! - A single deterministic [`clock::TimerWheel`] orders all events, with device
//!   start and keepalive staggered by a seeded RNG.
//! - A [`fault::FaultEngine`] injects `drop`, `delay`, `reorder`, `duplicate`,
//!   `half_packet` (TCP), `malformed` and `sip_error` faults, each with its own
//!   RNG stream.
//! - Devices and the platform reuse the real [`cheetah_gb28181_core`] SIP
//!   parser/encoder and the [`cheetah_gb28181_module`] XML builders/parsers, so
//!   the golden fixture and parser contracts are preserved.
//!
//! # Media boundary
//!
//! The simulator is a **control-plane** tool.  It only exercises signalling
//! handshakes (REGISTER, MESSAGE, INVITE/200/BYE) and produces media *control*
//! events.  It never generates, parses or transmits RTP, RTCP, PS, TS or ES
//! media payloads.

#![warn(missing_docs)]

pub mod clock;
pub mod device;
pub mod fault;
pub mod harness;
pub mod platform;
pub mod profile;
pub mod report;
pub mod rng;
pub mod scenario;
pub mod transport;
pub mod wire;

pub use harness::{Harness, run_scenario};
pub use report::RunReport;
pub use scenario::{Scenario, ScenarioError};
