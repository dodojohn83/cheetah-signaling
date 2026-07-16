#![doc = include_str!("../README.md")]

//! Cluster node registry and lease support for Cheetah Signaling.

pub mod compatibility;
pub mod error;
pub mod lease;

pub use compatibility::{CompatibilityError, CompatibilityMatrix};
pub use lease::NodeLeaseService;
