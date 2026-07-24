//! HTTP error mapping and RFC 9457 Problem Details.

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use cheetah_signal_types::SignalError;
use serde::Serialize;

/// Maximum byte length of an `HttpError` diagnostic message.
const MAX_HTTP_ERROR_BYTES: usize = 1024;

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

/// RFC 7807/9457 Problem Details response.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProblemDetails {
    /// Stable machine readable error code.
    pub code: String,
    /// Human readable message suitable for external callers.
    pub message: String,
    /// HTTP status code.
    pub status: u16,
    /// Request identifier to correlate with logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Field level violations, if any.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub field_violations: Vec<FieldViolation>,
}

/// Field level validation failure.
#[derive(Clone, Debug, Serialize)]
pub struct FieldViolation {
    /// Dot separated field path.
    pub field: String,
    /// Human readable description.
    pub description: String,
}

impl From<&cheetah_signal_types::FieldViolation> for FieldViolation {
    fn from(v: &cheetah_signal_types::FieldViolation) -> Self {
        Self {
            field: v.field.clone(),
            description: v.description.clone(),
        }
    }
}

/// Error type returned by HTTP handlers.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// A business error from the application/domain layer.
    #[error("business error: {0}")]
    Signal(#[from] SignalError),
    /// The request body could not be parsed.
    #[error("invalid request body: {0}")]
    InvalidJson(#[from] serde_json::Error),
    /// An authentication failure.
    #[error("unauthenticated: {0}")]
    Unauthenticated(String),
    /// An authorization failure.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The requested endpoint is not implemented.
    #[error("not implemented: {0}")]
    NotImplemented(String),
    /// The request exceeded its rate limit.
    #[error("rate limit exceeded: {0}")]
    RateLimited(String),
    /// A generic internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

impl HttpError {
    /// Creates an `Unauthenticated` error with a bounded message.
    pub fn unauthenticated(message: impl std::fmt::Display) -> Self {
        Self::Unauthenticated(clamp_str(&message.to_string(), MAX_HTTP_ERROR_BYTES))
    }

    /// Creates a `PermissionDenied` error with a bounded message.
    pub fn permission_denied(message: impl std::fmt::Display) -> Self {
        Self::PermissionDenied(clamp_str(&message.to_string(), MAX_HTTP_ERROR_BYTES))
    }

    /// Creates a `NotImplemented` error with a bounded message.
    pub fn not_implemented(message: impl std::fmt::Display) -> Self {
        Self::NotImplemented(clamp_str(&message.to_string(), MAX_HTTP_ERROR_BYTES))
    }

    /// Creates a `RateLimited` error with a bounded message.
    pub fn rate_limited(message: impl std::fmt::Display) -> Self {
        Self::RateLimited(clamp_str(&message.to_string(), MAX_HTTP_ERROR_BYTES))
    }

    /// Creates an `Internal` error with a bounded message.
    pub fn internal(message: impl std::fmt::Display) -> Self {
        Self::Internal(clamp_str(&message.to_string(), MAX_HTTP_ERROR_BYTES))
    }

    /// HTTP status code for this error.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Signal(e) => StatusCode::from_u16(e.to_http_status())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Self::InvalidJson(_) => StatusCode::BAD_REQUEST,
            Self::Unauthenticated(_) => StatusCode::UNAUTHORIZED,
            Self::PermissionDenied(_) => StatusCode::FORBIDDEN,
            Self::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Stable error code.
    pub fn code(&self) -> &str {
        match self {
            Self::Signal(e) => e.code(),
            Self::InvalidJson(_) => "INVALID_JSON",
            Self::Unauthenticated(_) => "UNAUTHENTICATED",
            Self::PermissionDenied(_) => "PERMISSION_DENIED",
            Self::NotImplemented(_) => "NOT_IMPLEMENTED",
            Self::RateLimited(_) => "RATE_LIMITED",
            Self::Internal(_) => "INTERNAL",
        }
    }

    /// Converts to a `ProblemDetails` value.
    pub fn to_problem(&self, request_id: Option<String>) -> ProblemDetails {
        let violations = match self {
            Self::Signal(e) => e
                .field_violations()
                .iter()
                .map(FieldViolation::from)
                .collect(),
            _ => Vec::new(),
        };
        let message = match self {
            Self::Internal(_) => "internal server error".to_string(),
            Self::RateLimited(_) => "rate limit exceeded".to_string(),
            _ => self.to_string(),
        };
        ProblemDetails {
            code: self.code().to_string(),
            message,
            status: self.status().as_u16(),
            request_id,
            field_violations: violations,
        }
    }
}

impl From<cheetah_storage_api::StorageError> for HttpError {
    fn from(err: cheetah_storage_api::StorageError) -> Self {
        let domain = cheetah_domain::DomainError::from(err);
        Self::Signal(domain.into())
    }
}

impl From<cheetah_domain::DomainError> for HttpError {
    fn from(err: cheetah_domain::DomainError) -> Self {
        Self::Signal(err.into())
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = self.status();
        let problem = self.to_problem(None);
        (status, Json(problem)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_message_is_clamped() {
        let long = "x".repeat(MAX_HTTP_ERROR_BYTES + 10);
        let err = HttpError::unauthenticated(long);
        if let HttpError::Unauthenticated(msg) = err {
            assert!(msg.len() <= MAX_HTTP_ERROR_BYTES);
            assert!(msg.starts_with("x"));
        } else {
            panic!("expected Unauthenticated variant");
        }
    }

    #[test]
    fn internal_message_is_clamped() {
        let long = "x".repeat(MAX_HTTP_ERROR_BYTES + 10);
        let err = HttpError::internal(format!("failed: {long}"));
        if let HttpError::Internal(msg) = err {
            assert!(msg.len() <= MAX_HTTP_ERROR_BYTES);
            assert!(msg.starts_with("failed:"));
        } else {
            panic!("expected Internal variant");
        }
    }

    #[test]
    fn clamp_respects_utf8_char_boundaries() {
        let text = "x".repeat(MAX_HTTP_ERROR_BYTES - 1) + "é";
        let err = HttpError::unauthenticated(text);
        if let HttpError::Unauthenticated(msg) = err {
            assert!(msg.len() <= MAX_HTTP_ERROR_BYTES);
            assert!(msg.ends_with('x'));
        } else {
            panic!("expected Unauthenticated variant");
        }
    }
}
