//! Media node registry, health tracking, and scheduling for Cheetah Signaling.

#![warn(missing_docs)]

pub mod config;
pub mod error;
pub mod grpc;
pub mod model;
pub mod port;
pub mod registry;
pub mod scheduler;
pub mod server;

pub use config::{MediaRegistryConfig, SchedulerConfig};
pub use error::SchedulerError;
pub use grpc::{MediaClusterRegistryService, PeerIdentity};
pub use model::{MediaCapability, MediaNode, MediaNodeCapacity, MediaNodeHealth, NodeStatus};
pub use port::SchedulerMediaPort;
pub use registry::{InMemoryMediaNodeRegistry, MediaNodeRegistry};
pub use scheduler::{LeastLoadedScheduler, MediaScheduler};
pub use server::GrpcServer;
