//! Media client errors.

use cheetah_domain::DomainError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a gRPC status message carried in `MediaClientError`.
const MAX_GRPC_STATUS_MESSAGE_BYTES: usize = 1024;
/// Maximum byte length of a fallible `MediaClientError` human-readable string.
const MAX_MEDIA_CLIENT_ERROR_BYTES: usize = 2048;

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
            | MediaClientError::InvalidDeadline(_) => DomainError::invalid_argument(clamp_str(
                &err.to_string(),
                MAX_MEDIA_CLIENT_ERROR_BYTES,
            )),
            MediaClientError::Grpc(ref status) => match status.code() {
                tonic::Code::InvalidArgument => DomainError::invalid_argument(clamp_str(
                    status.message(),
                    MAX_GRPC_STATUS_MESSAGE_BYTES,
                )),
                tonic::Code::NotFound => DomainError::not_found(
                    "media",
                    clamp_str(status.message(), MAX_GRPC_STATUS_MESSAGE_BYTES),
                ),
                tonic::Code::AlreadyExists => DomainError::invalid_argument(clamp_str(
                    status.message(),
                    MAX_GRPC_STATUS_MESSAGE_BYTES,
                )),
                _ => DomainError::unavailable(clamp_str(
                    &err.to_string(),
                    MAX_MEDIA_CLIENT_ERROR_BYTES,
                )),
            },
            MediaClientError::Transport(_)
            | MediaClientError::CircuitOpen(_)
            | MediaClientError::PoolExhausted(_)
            | MediaClientError::TlsConfig(_) => {
                DomainError::unavailable(clamp_str(&err.to_string(), MAX_MEDIA_CLIENT_ERROR_BYTES))
            }
        }
    }
}
