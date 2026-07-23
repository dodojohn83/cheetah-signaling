//! Errors returned by the ONVIF service layer.

use cheetah_onvif_core::OnvifError;
use thiserror::Error;

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
    /// Creates an invalid value error.
    pub fn invalid_value(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidValue {
            field: field.into(),
            message: message.into(),
        }
    }
}
