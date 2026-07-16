//! Device owner lease, resolver, and routing for Cheetah Signaling clusters.
#![doc = include_str!("../README.md")]

pub mod assignment;
pub mod error;
pub mod lease;

pub use assignment::{DeviceAssignmentService, RateLimitConfig};
pub use error::DeviceAssignmentError;
pub use lease::{CachingDeviceOwnerResolver, OwnerLeaseService};
