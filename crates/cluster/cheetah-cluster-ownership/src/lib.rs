//! Device owner lease, resolver, and routing for Cheetah Signaling clusters.
#![doc = include_str!("../README.md")]

pub mod assignment;
pub mod error;
pub mod lease;
pub mod rolling_upgrade;

pub use assignment::{DeviceAssignmentService, RateLimitConfig};
pub use error::{DeviceAssignmentError, RollingUpgradeError};
pub use lease::{CachingDeviceOwnerResolver, OwnerLeaseService};
pub use rolling_upgrade::{DeviceProtocolLookup, DrainReport, DrainingMigrationService};
