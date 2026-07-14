#![doc = include_str!("../README.md")]

//! Tokio runtime implementation for the Cheetah Signaling device runtime.

/// Bounded admission controller.
pub mod admission;
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
/// Runtime entry point.
pub use runtime::Runtime;
