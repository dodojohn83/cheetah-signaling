//! Errors for the plugin host.

use cheetah_plugin_sdk::PluginError;

/// Errors returned by the plugin host.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum PluginHostError {
    /// The plugin manifest is invalid or the checksum/signature failed.
    #[error("manifest validation failed: {0}")]
    InvalidManifest(String),
    /// The plugin's SDK version is incompatible with the host.
    #[error("incompatible SDK: {0}")]
    IncompatibleSdk(String),
    /// A plugin with the same name is already registered.
    #[error("plugin {0} is already registered")]
    AlreadyRegistered(String),
    /// A plugin instance with the same ID already exists.
    #[error("plugin instance {0} already exists")]
    InstanceExists(String),
    /// The requested plugin or instance was not found.
    #[error("plugin {0} not found")]
    NotFound(String),
    /// The driver reported an error.
    #[error("driver error: {0}")]
    Driver(PluginError),
    /// A lifecycle operation timed out.
    #[error("plugin host operation timed out")]
    Timeout,
    /// An internal host invariant was violated.
    #[error("internal host error: {0}")]
    Internal(String),
}

impl From<PluginError> for PluginHostError {
    fn from(err: PluginError) -> Self {
        match err {
            PluginError::InvalidManifest(msg) => Self::InvalidManifest(msg),
            PluginError::IncompatibleSdk { plugin, host } => {
                Self::IncompatibleSdk(format!("plugin requires {plugin}, host provides {host}"))
            }
            PluginError::InvalidChecksum => Self::InvalidManifest("checksum failed".to_string()),
            PluginError::UnsupportedProtocol(name) => {
                Self::InvalidManifest(format!("unsupported protocol {name}"))
            }
            other => Self::Driver(other),
        }
    }
}
