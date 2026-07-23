//! Driver error types.

use cheetah_onvif_core::OnvifError;
use cheetah_onvif_services::OnvifServiceError;

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

/// Result alias for the driver.
pub type DriverResult<T> = Result<T, DriverError>;
