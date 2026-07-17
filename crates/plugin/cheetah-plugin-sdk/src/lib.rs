//! Cheetah Signaling plugin SDK.
//!
//! Defines the contract between the plugin host and protocol plugins. Built-in
//! and out-of-process drivers both implement the same ports, so new protocols
//! can be added without modifying the domain core.
//!
//! The SDK intentionally does not expose database connection pools, NATS
//! clients, or global configuration. Drivers receive a [`DriverContext`] with
//! restricted capabilities: publishing events, requesting media sessions,
//! looking up secrets by reference, and registering endpoints.

#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod checksum;
pub mod driver;
pub mod error;
pub mod manifest;
#[cfg(test)]
mod tests;
pub mod version;

pub use checksum::verify_manifest_checksum;
pub use driver::{
    CapabilityDescriptor, CommandSource, DeviceSink, DriverCommand, DriverContext, HealthReport,
    HealthStatus, MonotonicSeconds, ProtocolDriver, ProtocolDriverFactory, ProtocolEvent,
};
pub use error::PluginError;
pub use manifest::{
    ConfigSchema, PluginChecksum, PluginEntry, PluginManifest, PluginName, PluginPermission,
    PluginVersion, ProtocolCapability, ProtocolDirection, ResourceBudget, SdkVersionReq,
};
pub use version::negotiate_sdk_version;
