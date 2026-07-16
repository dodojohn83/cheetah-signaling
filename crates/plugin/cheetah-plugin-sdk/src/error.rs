//! Stable error classification for plugin SDK operations.

/// Errors returned by plugin SDK validation, negotiation, and driver calls.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum PluginError {
    /// The manifest is malformed or missing required fields.
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    /// The plugin's SDK requirement does not match the host SDK.
    #[error("incompatible SDK: plugin requires {plugin}, host provides {host}")]
    IncompatibleSdk {
        /// SDK version required by the plugin.
        plugin: String,
        /// SDK version provided by the host.
        host: String,
    },
    /// The manifest checksum or signature verification failed.
    #[error("manifest checksum or signature verification failed")]
    InvalidChecksum,
    /// The plugin declares a protocol that the host does not support.
    #[error("unsupported protocol: {0}")]
    UnsupportedProtocol(String),
    /// The plugin's declared resource budget exceeds host limits.
    #[error("resource budget exceeded: {0}")]
    ResourceBudgetExceeded(String),
    /// The driver received a command or configuration it cannot handle.
    #[error("unsupported command or configuration: {0}")]
    Unsupported(String),
    /// The operation exceeded its deadline or was cancelled.
    #[error("operation cancelled or deadline exceeded")]
    Cancelled,
    /// A transient failure that may succeed on retry.
    #[error("transient driver error: {0}")]
    Transient(String),
    /// A non-retryable driver error.
    #[error("driver error: {0}")]
    Driver(String),
}
