//! Errors returned by the GB28181 protocol module.

use cheetah_gb28181_core::{DigestError, SipError};
use std::fmt;

/// Errors produced by the GB28181 module.
#[derive(Debug)]
pub enum Gb28181ModuleError {
    /// SIP parsing/encoding failure.
    Sip(SipError),
    /// Digest authentication failure.
    Digest(DigestError),
    /// Invalid device identifier.
    InvalidDeviceId(String),
    /// Malformed or unsupported SIP message.
    InvalidMessage(String),
    /// XML parsing/validation failure.
    Xml(String),
    /// Authentication required.
    Unauthorized,
    /// Unsupported method or capability for the current state.
    Unsupported,
    /// Runtime error from the actor context.
    Runtime(cheetah_runtime_api::RuntimeError),
}

impl fmt::Display for Gb28181ModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sip(e) => write!(f, "sip error: {e}"),
            Self::Digest(e) => write!(f, "digest error: {e}"),
            Self::InvalidDeviceId(s) => write!(f, "invalid device id: {s}"),
            Self::InvalidMessage(s) => write!(f, "invalid message: {s}"),
            Self::Xml(s) => write!(f, "xml error: {s}"),
            Self::Unauthorized => f.write_str("authentication required"),
            Self::Unsupported => f.write_str("unsupported operation"),
            Self::Runtime(e) => write!(f, "runtime error: {e}"),
        }
    }
}

impl std::error::Error for Gb28181ModuleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sip(e) => Some(e),
            Self::Digest(e) => Some(e),
            Self::Runtime(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SipError> for Gb28181ModuleError {
    fn from(value: SipError) -> Self {
        Self::Sip(value)
    }
}

impl From<DigestError> for Gb28181ModuleError {
    fn from(value: DigestError) -> Self {
        Self::Digest(value)
    }
}

impl From<cheetah_runtime_api::RuntimeError> for Gb28181ModuleError {
    fn from(value: cheetah_runtime_api::RuntimeError) -> Self {
        Self::Runtime(value)
    }
}

impl From<quick_xml::Error> for Gb28181ModuleError {
    fn from(value: quick_xml::Error) -> Self {
        Self::Xml(value.to_string())
    }
}

impl From<std::net::AddrParseError> for Gb28181ModuleError {
    fn from(value: std::net::AddrParseError) -> Self {
        Self::InvalidMessage(value.to_string())
    }
}
