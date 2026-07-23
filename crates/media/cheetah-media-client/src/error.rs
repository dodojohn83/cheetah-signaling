//! Media client errors.

use cheetah_domain::DomainError;

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
    /// The connection pool reached its configured limit.
    #[error("connection pool exhausted: {0}")]
    PoolExhausted(String),
    /// The endpoint scheme is plain HTTP but insecure HTTP is not allowed.
    #[error("insecure endpoint not allowed: {0}")]
    InsecureEndpoint(String),
    /// The endpoint resolves to or uses an internal network address.
    #[error("internal endpoint not allowed: {0}")]
    InternalEndpoint(String),
    /// TLS configuration is invalid.
    #[error("TLS configuration failed: {0}")]
    TlsConfig(String),
}

impl From<MediaClientError> for DomainError {
    fn from(err: MediaClientError) -> Self {
        match err {
            MediaClientError::InvalidEndpoint(_)
            | MediaClientError::InsecureEndpoint(_)
            | MediaClientError::InternalEndpoint(_)
            | MediaClientError::MissingIdentifier { .. }
            | MediaClientError::InvalidDeadline(_) => {
                DomainError::invalid_argument(err.to_string())
            }
            MediaClientError::Grpc(ref status) => match status.code() {
                tonic::Code::InvalidArgument => {
                    DomainError::invalid_argument(status.message().to_string())
                }
                tonic::Code::NotFound => {
                    DomainError::not_found("media", status.message().to_string())
                }
                tonic::Code::AlreadyExists => {
                    DomainError::invalid_argument(status.message().to_string())
                }
                _ => DomainError::unavailable(err.to_string()),
            },
            MediaClientError::Transport(_)
            | MediaClientError::CircuitOpen(_)
            | MediaClientError::PoolExhausted(_)
            | MediaClientError::TlsConfig(_) => DomainError::unavailable(err.to_string()),
        }
    }
}
