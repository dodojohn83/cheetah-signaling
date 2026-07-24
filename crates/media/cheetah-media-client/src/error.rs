//! Media client errors.

use cheetah_domain::DomainError;
use cheetah_signal_types::clamp_str;

/// Maximum byte length of a gRPC status message carried in `MediaClientError`.
const MAX_GRPC_STATUS_MESSAGE_BYTES: usize = 1024;
/// Maximum byte length of a fallible `MediaClientError` human-readable string.
const MAX_MEDIA_CLIENT_ERROR_BYTES: usize = 2048;
/// Maximum byte length of a `MissingIdentifier` reason string.
const MAX_MEDIA_CLIENT_REASON_BYTES: usize = 256;

/// Errors returned by the media control client.
#[derive(Debug, thiserror::Error)]
pub enum MediaClientError {
    /// The media node endpoint is not known or invalid.
    #[error("invalid or unknown endpoint: {0}")]
    InvalidEndpoint(String),
    /// The gRPC call failed.
    #[error("gRPC call failed: {0}")]
    Grpc(tonic::Status),
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

impl MediaClientError {
    /// Creates an `InvalidEndpoint` error with a bounded message.
    pub fn invalid_endpoint(endpoint: impl std::fmt::Display) -> Self {
        Self::InvalidEndpoint(clamp_str(
            &endpoint.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates an `InsecureEndpoint` error with a bounded message.
    pub fn insecure_endpoint(endpoint: impl std::fmt::Display) -> Self {
        Self::InsecureEndpoint(clamp_str(
            &endpoint.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates an `InternalEndpoint` error with a bounded message.
    pub fn internal_endpoint(endpoint: impl std::fmt::Display) -> Self {
        Self::InternalEndpoint(clamp_str(
            &endpoint.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates an `InvalidDeadline` error with a bounded message.
    pub fn invalid_deadline(deadline: impl std::fmt::Display) -> Self {
        Self::InvalidDeadline(clamp_str(
            &deadline.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates a `PoolExhausted` error with a bounded message.
    pub fn pool_exhausted(endpoint: impl std::fmt::Display) -> Self {
        Self::PoolExhausted(clamp_str(
            &endpoint.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates a `CircuitOpen` error with a bounded message.
    pub fn circuit_open(endpoint: impl std::fmt::Display) -> Self {
        Self::CircuitOpen(clamp_str(
            &endpoint.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates a `TlsConfig` error with a bounded message.
    pub fn tls_config(message: impl std::fmt::Display) -> Self {
        Self::TlsConfig(clamp_str(
            &message.to_string(),
            MAX_MEDIA_CLIENT_ERROR_BYTES,
        ))
    }

    /// Creates a `MissingIdentifier` error with a bounded reason.
    pub fn missing_identifier(field: &'static str, reason: impl std::fmt::Display) -> Self {
        Self::MissingIdentifier {
            field,
            reason: clamp_str(&reason.to_string(), MAX_MEDIA_CLIENT_REASON_BYTES),
        }
    }

    /// Creates a `Grpc` error with a clamped status message.
    pub fn grpc(status: tonic::Status) -> Self {
        let message = clamp_str(status.message(), MAX_GRPC_STATUS_MESSAGE_BYTES);
        Self::Grpc(tonic::Status::new(status.code(), message))
    }
}

impl From<tonic::Status> for MediaClientError {
    fn from(status: tonic::Status) -> Self {
        Self::grpc(status)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grpc_status_message_is_clamped() {
        let long = "x".repeat(MAX_GRPC_STATUS_MESSAGE_BYTES + 10);
        let status = tonic::Status::invalid_argument(long.clone());
        let err = MediaClientError::grpc(status);
        if let MediaClientError::Grpc(clamped) = err {
            assert!(clamped.message().len() <= MAX_GRPC_STATUS_MESSAGE_BYTES);
            assert!(clamped.message().starts_with("x"));
        } else {
            panic!("expected Grpc variant");
        }
    }

    #[test]
    fn invalid_endpoint_message_is_clamped() {
        let long = "x".repeat(MAX_MEDIA_CLIENT_ERROR_BYTES + 10);
        let err = MediaClientError::invalid_endpoint(long);
        if let MediaClientError::InvalidEndpoint(msg) = err {
            assert!(msg.len() <= MAX_MEDIA_CLIENT_ERROR_BYTES);
        } else {
            panic!("expected InvalidEndpoint variant");
        }
    }

    #[test]
    fn missing_identifier_reason_is_clamped() {
        let long = "x".repeat(MAX_MEDIA_CLIENT_REASON_BYTES + 10);
        let err = MediaClientError::missing_identifier("field", long);
        if let MediaClientError::MissingIdentifier { field, reason } = err {
            assert_eq!(field, "field");
            assert!(reason.len() <= MAX_MEDIA_CLIENT_REASON_BYTES);
        } else {
            panic!("expected MissingIdentifier variant");
        }
    }

    #[test]
    fn clamp_respects_utf8_char_boundaries() {
        let text = "x".repeat(MAX_MEDIA_CLIENT_ERROR_BYTES - 1) + "é";
        let err = MediaClientError::invalid_endpoint(text);
        if let MediaClientError::InvalidEndpoint(msg) = err {
            assert!(msg.len() <= MAX_MEDIA_CLIENT_ERROR_BYTES);
            assert!(msg.ends_with('x'));
        } else {
            panic!("expected InvalidEndpoint variant");
        }
    }
}
