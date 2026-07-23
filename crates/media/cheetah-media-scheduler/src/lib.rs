//! Media node registry, health tracking, and scheduling for Cheetah Signaling.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod event_consumer;
pub(crate) mod event_consumer_support;
pub mod grpc;
pub mod metrics;
pub mod model;
pub mod persistent_registry;
pub mod port;
pub mod registry;
pub mod scheduler;

pub use config::{MediaEventConsumerConfig, MediaRegistryConfig, SchedulerConfig};
pub use error::SchedulerError;
pub use event_consumer::{MediaEventConsumer, NoopReconciliationHandler, ReconciliationHandler};
pub use grpc::{MediaClusterRegistryService, PeerIdentity};
pub use metrics::MediaMetrics;
pub use model::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
pub use persistent_registry::PersistentMediaNodeRegistry;
pub use port::SchedulerMediaPort;
pub use registry::{InMemoryMediaNodeRegistry, MediaNodeRegistry};
pub use scheduler::{LeastLoadedScheduler, MediaScheduler};
