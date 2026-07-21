#![doc = include_str!("../README.md")]

//! Tokio runtime implementation for the Cheetah Signaling device runtime.

/// Bounded admission controller.
pub mod admission;
/// GB28181 runtime/application metrics aggregator.
pub mod gb_metrics;
/// Runtime readiness and degraded-state health reporting.
pub mod health;
/// Runtime entry point.
pub mod runtime;
/// Fixed shard worker.
pub mod shard;
/// Hierarchical timer wheel.
pub mod timer_wheel;

pub(crate) mod system_clock;
pub(crate) mod timer_scheduler;

/// Bounded admission controller.
pub use admission::AdmissionController;
/// GB28181 metrics aggregator.
pub use gb_metrics::GbMetrics;
/// Runtime health types.
pub use health::{HealthReason, HealthThresholds, RuntimeHealth, RuntimeHealthSource};
/// Runtime entry point.
pub use runtime::Runtime;
