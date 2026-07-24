//! Error types and mapping for Cheetah Signaling.

use crate::CorrelationId;

/// Maximum byte length of a `SignalError` safe message.
const MAX_ERROR_MESSAGE_BYTES: usize = 2048;
/// Maximum byte length of a field violation field path.
const MAX_FIELD_VIOLATION_FIELD_BYTES: usize = 256;
/// Maximum byte length of a field violation description.
const MAX_FIELD_VIOLATION_DESCRIPTION_BYTES: usize = 1024;

/// Truncates `s` to at most `max` bytes, never splitting a multi-byte character.
pub fn clamp_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    s[..end].to_string()
}

/// Truncates a owned string to at most `max` bytes, never splitting a multi-byte character.
pub fn clamp_string_bytes(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    clamp_str(&s, max)
}

/// Categorization of failures that can be returned to callers.
///
/// Each variant has a stable code string and a retryability classification.
/// Callers must not rely on the human readable message to identify errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SignalErrorKind {
    /// The request is malformed or invalid.
    InvalidArgument,
    /// The caller could not be authenticated.
    Unauthenticated,
    /// The caller is not authorized to perform the action.
    PermissionDenied,
    /// The requested resource does not exist.
    NotFound,
    /// The resource already exists.
    AlreadyExists,
    /// The resource state conflicts with the request.
    Conflict,
    /// The owner epoch has moved on, likely due to reassignment.
    StaleOwner,
    /// The resource is busy and the operation cannot be performed now.
    Busy,
    /// The caller has exceeded a rate limit.
    RateLimited,
    /// The operation timed out.
    Timeout,
    /// The operation was cancelled.
    Cancelled,
    /// A required service or dependency is unavailable.
    Unavailable,
    /// The requested capability is not supported.
    Unsupported,
    /// A version mismatch was detected.
    VersionMismatch,
    /// A precondition such as If-Match revision failed.
    FailedPrecondition,
    /// The supplied pagination cursor has expired or is no longer valid.
    CursorExpired,
    /// A protocol level operation failed.
    ProtocolFailed,
    /// A media operation failed.
    MediaFailed,
    /// A storage operation failed.
    StorageFailed,
    /// An internal error occurred.
    Internal,
}

impl SignalErrorKind {
    /// Stable machine readable code for this error.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "INVALID_ARGUMENT",
            Self::Unauthenticated => "UNAUTHENTICATED",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::NotFound => "NOT_FOUND",
            Self::AlreadyExists => "ALREADY_EXISTS",
            Self::Conflict => "CONFLICT",
            Self::StaleOwner => "STALE_OWNER",
            Self::Busy => "BUSY",
            Self::RateLimited => "RATE_LIMITED",
            Self::Timeout => "TIMEOUT",
            Self::Cancelled => "CANCELLED",
            Self::Unavailable => "UNAVAILABLE",
            Self::Unsupported => "UNSUPPORTED",
            Self::VersionMismatch => "VERSION_MISMATCH",
            Self::FailedPrecondition => "FAILED_PRECONDITION",
            Self::CursorExpired => "CURSOR_EXPIRED",
            Self::ProtocolFailed => "PROTOCOL_FAILED",
            Self::MediaFailed => "MEDIA_FAILED",
            Self::StorageFailed => "STORAGE_FAILED",
            Self::Internal => "INTERNAL",
        }
    }

    /// Whether the operation can be retried without modification.
    pub const fn is_retryable(self) -> bool {
        matches!(
            self,
            Self::StaleOwner
                | Self::Busy
                | Self::RateLimited
                | Self::Timeout
                | Self::Unavailable
                | Self::ProtocolFailed
                | Self::StorageFailed
                | Self::MediaFailed
        )
    }

    /// HTTP status code that should be returned for this error.
    pub const fn to_http_status(self) -> u16 {
        match self {
            Self::InvalidArgument => 400,
            Self::Unauthenticated => 401,
            Self::PermissionDenied => 403,
            Self::NotFound => 404,
            Self::AlreadyExists => 409,
            Self::Conflict => 409,
            Self::StaleOwner => 409,
            Self::Busy => 429,
            Self::RateLimited => 429,
            Self::Timeout => 504,
            Self::Cancelled => 499,
            Self::Unavailable => 503,
            Self::Unsupported => 501,
            Self::VersionMismatch => 409,
            Self::FailedPrecondition => 412,
            Self::CursorExpired => 400,
            Self::ProtocolFailed => 502,
            Self::MediaFailed => 502,
            Self::StorageFailed => 503,
            Self::Internal => 500,
        }
    }

    /// gRPC status code equivalent for this error.
    pub const fn to_grpc_code(self) -> i32 {
        match self {
            Self::InvalidArgument => 3,
            Self::Unauthenticated => 16,
            Self::PermissionDenied => 7,
            Self::NotFound => 5,
            Self::AlreadyExists => 6,
            Self::Conflict => 6,
            Self::StaleOwner => 9,
            Self::Busy => 14,
            Self::RateLimited => 8,
            Self::Timeout => 4,
            Self::Cancelled => 1,
            Self::Unavailable => 14,
            Self::Unsupported => 12,
            Self::VersionMismatch => 9,
            Self::FailedPrecondition => 9,
            Self::CursorExpired => 3,
            Self::ProtocolFailed => 14,
            Self::MediaFailed => 14,
            Self::StorageFailed => 14,
            Self::Internal => 13,
        }
    }
}

/// A structured, safe business error.
///
/// The [`Display`] implementation only exposes the safe `message`.
/// The internal `source` chain is kept for diagnostics but is not part of the
/// public wire format.
#[derive(Debug, thiserror::Error)]
#[error("{message} (code: {code})")]
pub struct SignalError {
    kind: SignalErrorKind,
    code: &'static str,
    message: String,
    retryable: bool,
    field_violations: Vec<FieldViolation>,
    correlation_id: Option<CorrelationId>,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl SignalError {
    /// Creates a new error with the given kind and safe message.
    pub fn new(kind: SignalErrorKind, message: impl Into<String>) -> Self {
        let message = clamp_string_bytes(message.into(), MAX_ERROR_MESSAGE_BYTES);
        Self {
            code: kind.code(),
            retryable: kind.is_retryable(),
            kind,
            message,
            field_violations: Vec::new(),
            correlation_id: None,
            source: None,
        }
    }

    /// The kind of error.
    pub fn kind(&self) -> SignalErrorKind {
        self.kind
    }

    /// Stable code for this error.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Safe, non sensitive message suitable for external responses.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Whether the operation should be retried.
    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Field level violations, if any.
    pub fn field_violations(&self) -> &[FieldViolation] {
        &self.field_violations
    }

    /// Correlation ID attached to this error.
    pub fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    /// Adds a field violation.
    #[must_use]
    pub fn with_field_violation(
        mut self,
        field: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.field_violations.push(FieldViolation {
            field: clamp_string_bytes(field.into(), MAX_FIELD_VIOLATION_FIELD_BYTES),
            description: clamp_string_bytes(
                description.into(),
                MAX_FIELD_VIOLATION_DESCRIPTION_BYTES,
            ),
        });
        self
    }

    /// Sets the correlation id.
    #[must_use]
    pub fn with_correlation_id(mut self, correlation_id: CorrelationId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Attaches an internal cause to the error.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// HTTP status code appropriate for this error.
    pub fn to_http_status(&self) -> u16 {
        self.kind.to_http_status()
    }

    /// gRPC status code appropriate for this error.
    pub fn to_grpc_code(&self) -> i32 {
        self.kind.to_grpc_code()
    }
}

/// A validation failure for a particular field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldViolation {
    /// Dot separated path to the field.
    pub field: String,
    /// Human readable description.
    pub description: String,
}

/// Alias used throughout the workspace for fallible operations.
pub type Result<T> = std::result::Result<T, SignalError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_error_clamps_oversized_message() {
        let oversized = "x".repeat(MAX_ERROR_MESSAGE_BYTES + 1);
        let err = SignalError::new(SignalErrorKind::InvalidArgument, oversized);
        assert_eq!(err.message().len(), MAX_ERROR_MESSAGE_BYTES);
    }

    #[test]
    fn signal_error_clamps_oversized_field_violation() {
        let err = SignalError::new(SignalErrorKind::InvalidArgument, "bad request")
            .with_field_violation(
                "x".repeat(MAX_FIELD_VIOLATION_FIELD_BYTES + 1),
                "y".repeat(5),
            );
        assert_eq!(
            err.field_violations()[0].field.len(),
            MAX_FIELD_VIOLATION_FIELD_BYTES
        );
        assert_eq!(err.field_violations()[0].description, "y".repeat(5));
    }

    #[test]
    fn signal_error_clamps_oversized_field_description() {
        let err = SignalError::new(SignalErrorKind::InvalidArgument, "bad request")
            .with_field_violation(
                "field",
                "x".repeat(MAX_FIELD_VIOLATION_DESCRIPTION_BYTES + 1),
            );
        assert_eq!(
            err.field_violations()[0].description.len(),
            MAX_FIELD_VIOLATION_DESCRIPTION_BYTES
        );
    }

    #[test]
    fn clamp_string_bytes_preserves_utf8_boundary() {
        let s = "α".repeat(10);
        let clamped = super::clamp_string_bytes(s.clone(), 3);
        assert!(clamped.is_char_boundary(clamped.len()));
        assert!(clamped.len() <= 3);
    }
}
