//! Media client errors.

/// Errors returned by the media control client.
#[derive(Debug, thiserror::Error)]
pub enum MediaClientError {
    /// The media node endpoint is not known or invalid.
    #[error("invalid or unknown endpoint: {0}")]
    InvalidEndpoint(String),
    /// The gRPC call failed.
    #[error("gRPC call failed: {0}")]
    Grpc(#[from] tonic::Status),
    /// Transport setup failed.
    #[error("transport error: {0}")]
    Transport(#[from] tonic::transport::Error),
    /// A required identifier was missing.
    #[error("missing {field}: {reason}")]
    MissingIdentifier {
        /// Field name.
        field: &'static str,
        /// Reason.
        reason: String,
    },
    /// The circuit breaker is open for the target node.
    #[error("circuit breaker open for endpoint: {0}")]
    CircuitOpen(String),
    /// The deadline could not be encoded.
    #[error("invalid deadline: {0}")]
    InvalidDeadline(String),
}
