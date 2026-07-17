//! Cheetah Signaling plugin host.
//!
//! Loads and manages built-in protocol drivers using the shared plugin SDK
//! ports. The host validates manifests, negotiates SDK versions, and handles
//! driver lifecycle (start, drain, shutdown, health) with bounded timeouts.

#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod error;
pub mod host;
pub mod loader;
pub mod registry;

pub use error::PluginHostError;
pub use host::{HostDriverContext, NoopSecretProvider, PluginHost, SecretProvider};
pub use loader::ManifestLoader;
pub use registry::BuiltInRegistry;
