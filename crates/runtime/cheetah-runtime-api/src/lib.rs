#![doc = include_str!("../README.md")]

//! Sans-I/O runtime API for the Cheetah Signaling device runtime.

/// Admission control port.
pub use admission::AdmissionController;
/// Runtime configuration.
pub use config::RuntimeConfig;
/// Device actor API and actor context.
pub use device_actor::{ActorContext, DeviceActor};
/// Runtime error types.
pub use error::RuntimeError;
/// Stable key and identifier types.
pub use keys::{DeviceKey, SessionKey, TimerId};
/// Runtime health metrics.
pub use metrics::{RuntimeMetrics, RuntimeMetricsSnapshot};
/// Runtime messages.
pub use runtime_message::RuntimeMessage;
/// Timer scheduler port.
pub use scheduler::Scheduler;
/// Protocol session registry.
pub use session_registry::SessionRegistry;
/// Shard router.
pub use shard_router::ShardRouter;

/// Admission control port.
pub mod admission;
/// Runtime configuration.
pub mod config;
/// Device actor API and actor context.
pub mod device_actor;
/// Runtime error types.
pub mod error;
/// Stable key and identifier types.
pub mod keys;
/// Runtime health metrics.
pub mod metrics;
/// Runtime messages.
pub mod runtime_message;
/// Timer scheduler port.
pub mod scheduler;
/// Protocol session registry.
pub mod session_registry;
/// Shard router.
pub mod shard_router;
