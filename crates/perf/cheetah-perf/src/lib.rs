//! Performance, durability and stress scenario tests for Cheetah Signaling.
//!
//! All scenarios live under `tests/` and are marked `#[ignore]` so they do not
//! run in normal `cargo nextest run --workspace` invocations. Run them
//! manually with `cargo test --package cheetah-perf -- --ignored` or with
//! `cargo nextest run -p cheetah-perf --run-ignored all` after reading the
//! scenario prerequisites and expected environment.
