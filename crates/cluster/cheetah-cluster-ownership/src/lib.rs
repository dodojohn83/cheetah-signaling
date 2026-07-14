//! Device owner lease, resolver, and routing for Cheetah Signaling clusters.
#![doc = include_str!("../README.md")]

pub mod lease;

pub use lease::{CachingDeviceOwnerResolver, OwnerLeaseService};
