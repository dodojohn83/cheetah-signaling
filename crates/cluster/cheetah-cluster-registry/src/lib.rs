#![doc = include_str!("../README.md")]

//! Cluster node registry and lease support for Cheetah Signaling.

pub mod error;
pub mod lease;

pub use lease::NodeLeaseService;
