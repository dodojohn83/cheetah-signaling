//! Errors returned by the ONVIF service layer.

use cheetah_onvif_core::OnvifError;
use cheetah_signal_types::clamp_str;
use thiserror::Error;

/// Maximum byte length of an `OnvifServiceError` diagnostic message or field name.
const MAX_ONVIF_SERVICE_ERROR_BYTES: usize = 1024;
const MAX_ONVIF_SERVICE_FIELD_BYTES: usize = 256;

fn clamp_message(message: impl std::fmt::Display) -> String {
    clamp_str(&message.to_string(), MAX_ONVIF_SERVICE_ERROR_BYTES)
}

fn clamp_field(field: impl std::fmt::Display) -> String {
    clamp_str(&field.to_string(), MAX_ONVIF_SERVICE_FIELD_BYTES)
}

/// An error produced by the ONVIF module.
#[derive(Debug, Error)]
pub enum OnvifServiceError {
    /// Underlying ONVIF wire error.
    #[error("onvif: {0}")]
    Onvif(#[from] OnvifError),
    /// A required field was missing in the response.
    #[error("missing field: {0}")]
    MissingField(String),
    /// A value was out of the expected range.
    #[error("invalid value for {field}: {message}")]
    InvalidValue {
        /// Field name.
        field: String,
        /// Human-readable message.
        message: String,
    },
    /// An unsupported capability or service version was requested.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The operation was rejected by policy.
    #[error("policy: {0}")]
    Policy(String),
}

impl From<std::io::Error> for OnvifServiceError {
    fn from(value: std::io::Error) -> Self {
        Self::Onvif(OnvifError::from(value))
    }
}

impl From<quick_xml::Error> for OnvifServiceError {
    fn from(value: quick_xml::Error) -> Self {
        Self::Onvif(OnvifError::from(value))
    }
}

impl From<std::string::FromUtf8Error> for OnvifServiceError {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::Onvif(OnvifError::from(value))
    }
}

impl From<url::ParseError> for OnvifServiceError {
    fn from(value: url::ParseError) -> Self {
        Self::Onvif(OnvifError::from(value))
    }
}

impl OnvifServiceError {
    /// Creates an `Onvif` error wrapping a bounded `OnvifError`.
    pub fn onvif(e: impl Into<OnvifError>) -> Self {
        Self::Onvif(e.into())
    }

    /// Creates a `MissingField` error with a bounded field name.
    pub fn missing_field(field: impl std::fmt::Display) -> Self {
        Self::MissingField(clamp_field(field))
    }

    /// Creates an `InvalidValue` error with bounded field and message.
    pub fn invalid_value(field: impl std::fmt::Display, message: impl std::fmt::Display) -> Self {
        Self::InvalidValue {
            field: clamp_field(field),
            message: clamp_message(message),
        }
    }

    /// Creates an `Unsupported` error with a bounded message.
    pub fn unsupported(message: impl std::fmt::Display) -> Self {
        Self::Unsupported(clamp_message(message))
    }

    /// Creates a `Policy` error with a bounded message.
    pub fn policy(message: impl std::fmt::Display) -> Self {
        Self::Policy(clamp_message(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_error_message_is_clamped() {
        let long = "x".repeat(MAX_ONVIF_SERVICE_ERROR_BYTES + 100);
        let err = OnvifServiceError::invalid_value("field", format!("message: {long}"));
        if let OnvifServiceError::InvalidValue { field, message } = err {
            assert_eq!(field.len(), 5);
            assert_eq!(message.len(), MAX_ONVIF_SERVICE_ERROR_BYTES);
        } else {
            panic!("expected InvalidValue variant");
        }
    }

    #[test]
    fn service_error_field_is_clamped() {
        let long_field = "x".repeat(MAX_ONVIF_SERVICE_FIELD_BYTES + 10);
        let err = OnvifServiceError::invalid_value(long_field, "short");
        if let OnvifServiceError::InvalidValue { field, .. } = err {
            assert_eq!(field.len(), MAX_ONVIF_SERVICE_FIELD_BYTES);
        } else {
            panic!("expected InvalidValue variant");
        }
    }
}
