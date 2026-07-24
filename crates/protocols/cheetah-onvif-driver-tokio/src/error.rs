//! Driver error types.

use cheetah_onvif_core::OnvifError;
use cheetah_onvif_services::OnvifServiceError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a `DriverError` diagnostic message.
const MAX_DRIVER_ERROR_BYTES: usize = 1024;

fn clamp_message(message: impl std::fmt::Display) -> String {
    clamp_str(&message.to_string(), MAX_DRIVER_ERROR_BYTES)
}

/// Errors produced by the Tokio ONVIF driver.
#[derive(Debug, thiserror::Error)]
pub enum DriverError {
    /// Underlying ONVIF core error.
    #[error(transparent)]
    Onvif(#[from] OnvifError),
    /// Module builder/parser error.
    #[error(transparent)]
    Module(#[from] OnvifServiceError),
    /// HTTP transport failure.
    #[error("http error: {0}")]
    Http(String),
    /// HTTP non-success status.
    #[error("http status {status}: {body}")]
    HttpStatus {
        /// Status code.
        status: u16,
        /// Truncated response body for diagnostics (no secrets expected).
        body: String,
    },
    /// Response body exceeded the configured limit.
    #[error("response body limit exceeded ({limit} bytes)")]
    BodyLimit {
        /// Configured limit.
        limit: usize,
    },
    /// Socket bind/send/recv failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Operation cancelled or timed out.
    #[error("timeout: {0}")]
    Timeout(String),
    /// Invalid configuration.
    #[error("config error: {0}")]
    Config(String),
    /// Driver capacity/overload (e.g. too many concurrent device endpoints).
    #[error("overloaded: {0}")]
    Overloaded(String),
}

impl DriverError {
    /// Creates an `Http` error with a bounded diagnostic message.
    pub fn http(message: impl std::fmt::Display) -> Self {
        Self::Http(clamp_message(message))
    }

    /// Creates a `Timeout` error with a bounded diagnostic message.
    pub fn timeout(message: impl std::fmt::Display) -> Self {
        Self::Timeout(clamp_message(message))
    }

    /// Creates a `Config` error with a bounded diagnostic message.
    pub fn config(message: impl std::fmt::Display) -> Self {
        Self::Config(clamp_message(message))
    }

    /// Creates an `Overloaded` error with a bounded diagnostic message.
    pub fn overloaded(message: impl std::fmt::Display) -> Self {
        Self::Overloaded(clamp_message(message))
    }
}

/// Result alias for the driver.
pub type DriverResult<T> = Result<T, DriverError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_error_message_is_clamped() {
        let long = "x".repeat(MAX_DRIVER_ERROR_BYTES + 100);
        let err = DriverError::http(format!("http failure: {long}"));
        if let DriverError::Http(msg) = err {
            assert_eq!(msg.len(), MAX_DRIVER_ERROR_BYTES);
        } else {
            panic!("expected Http variant");
        }
    }

    #[test]
    fn driver_timeout_message_is_clamped() {
        let long = "x".repeat(MAX_DRIVER_ERROR_BYTES + 100);
        let err = DriverError::timeout(format!("timeout: {long}"));
        if let DriverError::Timeout(msg) = err {
            assert_eq!(msg.len(), MAX_DRIVER_ERROR_BYTES);
        } else {
            panic!("expected Timeout variant");
        }
    }
}
