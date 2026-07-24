//! Stable error classification for plugin SDK operations.

/// Maximum byte length for a `PluginError` diagnostic message.
const MAX_PLUGIN_ERROR_BYTES: usize = 1024;

/// Maximum byte length for a version string carried by a `PluginError`.
const MAX_PLUGIN_ERROR_VERSION_BYTES: usize = 256;

/// Maximum byte length for a protocol or resource identifier carried by a `PluginError`.
const MAX_PLUGIN_ERROR_IDENTIFIER_BYTES: usize = 256;

/// Truncates `s` to a UTF-8-safe prefix of at most `max` bytes.
fn clamp_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

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

impl PluginError {
    /// Creates an `InvalidManifest` error with a bounded diagnostic message.
    pub fn invalid_manifest(message: impl std::fmt::Display) -> Self {
        Self::InvalidManifest(clamp_str(&message.to_string(), MAX_PLUGIN_ERROR_BYTES))
    }

    /// Creates an `IncompatibleSdk` error with bounded version strings.
    pub fn incompatible_sdk(
        plugin: impl std::fmt::Display,
        host: impl std::fmt::Display,
    ) -> Self {
        Self::IncompatibleSdk {
            plugin: clamp_str(&plugin.to_string(), MAX_PLUGIN_ERROR_VERSION_BYTES),
            host: clamp_str(&host.to_string(), MAX_PLUGIN_ERROR_VERSION_BYTES),
        }
    }

    /// Creates an `UnsupportedProtocol` error with a bounded protocol name.
    pub fn unsupported_protocol(protocol: impl std::fmt::Display) -> Self {
        Self::UnsupportedProtocol(clamp_str(
            &protocol.to_string(),
            MAX_PLUGIN_ERROR_IDENTIFIER_BYTES,
        ))
    }

    /// Creates a `ResourceBudgetExceeded` error with a bounded resource name.
    pub fn resource_budget_exceeded(resource: impl std::fmt::Display) -> Self {
        Self::ResourceBudgetExceeded(clamp_str(
            &resource.to_string(),
            MAX_PLUGIN_ERROR_IDENTIFIER_BYTES,
        ))
    }

    /// Creates an `Unsupported` error with a bounded diagnostic message.
    pub fn unsupported(message: impl std::fmt::Display) -> Self {
        Self::Unsupported(clamp_str(&message.to_string(), MAX_PLUGIN_ERROR_BYTES))
    }

    /// Creates a `Transient` error with a bounded diagnostic message.
    pub fn transient(message: impl std::fmt::Display) -> Self {
        Self::Transient(clamp_str(&message.to_string(), MAX_PLUGIN_ERROR_BYTES))
    }

    /// Creates a `Driver` error with a bounded diagnostic message.
    pub fn driver(message: impl std::fmt::Display) -> Self {
        Self::Driver(clamp_str(&message.to_string(), MAX_PLUGIN_ERROR_BYTES))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_error_message_is_clamped() {
        let long = "x".repeat(MAX_PLUGIN_ERROR_BYTES + 100);
        let err = PluginError::driver(format!("driver failure: {long}"));
        if let PluginError::Driver(msg) = err {
            assert_eq!(msg.len(), MAX_PLUGIN_ERROR_BYTES);
        } else {
            panic!("expected Driver variant");
        }
    }

    #[test]
    fn incompatible_sdk_versions_are_clamped() {
        let long = "x".repeat(MAX_PLUGIN_ERROR_VERSION_BYTES + 10);
        let err = PluginError::incompatible_sdk(format!("{long}"), "0.1.0");
        if let PluginError::IncompatibleSdk { plugin, host } = err {
            assert_eq!(plugin.len(), MAX_PLUGIN_ERROR_VERSION_BYTES);
            assert_eq!(host, "0.1.0");
        } else {
            panic!("expected IncompatibleSdk variant");
        }
    }

    #[test]
    fn clamp_respects_utf8_char_boundaries() {
        let text = "x".repeat(MAX_PLUGIN_ERROR_BYTES - 1) + "é";
        let err = PluginError::invalid_manifest(text);
        if let PluginError::InvalidManifest(msg) = err {
            assert!(msg.len() <= MAX_PLUGIN_ERROR_BYTES);
            assert!(msg.is_char_boundary(msg.len()));
        } else {
            panic!("expected InvalidManifest variant");
        }
    }
}
