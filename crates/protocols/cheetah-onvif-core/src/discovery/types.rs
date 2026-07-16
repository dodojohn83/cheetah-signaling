//! WS-Discovery data types.

use crate::error::{OnvifError, OnvifResult};

/// Stable endpoint reference, typically a UUID URN.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct EndpointReference(pub String);

/// Transport addresses advertised by a device.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct XAddrs(pub Vec<String>);

impl XAddrs {
    /// Parses a space-separated `XAddrs` string.
    pub fn parse(s: &str) -> OnvifResult<Self> {
        Ok(Self(s.split_whitespace().map(|s| s.to_string()).collect()))
    }
}

/// Discovery scopes as a space-separated list of URIs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Scopes(pub Vec<String>);

/// Metadata version used to detect changes between announcements.
pub type MetadataVersion = u64;

/// A WS-Discovery `Probe` request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Probe {
    /// Application message ID.
    pub app_id: String,
    /// Required device type list.
    pub types: Vec<String>,
    /// Optional scopes.
    pub scopes: Option<Scopes>,
}

/// One matched endpoint in a `ProbeMatches` response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeMatch {
    /// Endpoint reference.
    pub endpoint_reference: EndpointReference,
    /// Device types.
    pub types: Vec<String>,
    /// Discovery scopes.
    pub scopes: Option<Scopes>,
    /// Transport addresses.
    pub x_addrs: XAddrs,
    /// Metadata version.
    pub metadata_version: MetadataVersion,
    /// Timestamp at which the match was received.
    pub discovered_at: u64,
}

/// A collection of probe matches.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProbeMatches {
    /// Correlation ID (MessageID of the original `Probe`).
    pub relates_to: String,
    /// Matched endpoints.
    pub matches: Vec<ProbeMatch>,
}

/// A WS-Discovery `Resolve` request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Resolve {
    /// Application message ID.
    pub app_id: String,
    /// Endpoint reference to resolve.
    pub endpoint_reference: EndpointReference,
}

/// A single `Resolve` match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveMatch {
    /// Endpoint reference.
    pub endpoint_reference: EndpointReference,
    /// Transport addresses.
    pub x_addrs: XAddrs,
    /// Metadata version.
    pub metadata_version: MetadataVersion,
    /// Timestamp at which the match was received.
    pub discovered_at: u64,
}

/// A collection of resolve matches.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolveMatches {
    /// Correlation ID.
    pub relates_to: String,
    /// Matched endpoints.
    pub matches: Vec<ResolveMatch>,
}

/// A WS-Discovery `Hello` announcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hello {
    /// Endpoint reference.
    pub endpoint_reference: EndpointReference,
    /// Device types.
    pub types: Vec<String>,
    /// Discovery scopes.
    pub scopes: Option<Scopes>,
    /// Transport addresses.
    pub x_addrs: XAddrs,
    /// Metadata version.
    pub metadata_version: MetadataVersion,
    /// Timestamp at which the announcement was received.
    pub discovered_at: u64,
}

/// A WS-Discovery `Bye` announcement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bye {
    /// Endpoint reference that is leaving.
    pub endpoint_reference: EndpointReference,
    /// Timestamp at which the announcement was received.
    pub discovered_at: u64,
}

/// Validates and returns a cleaned EPR string.
pub(crate) fn validate_epr(s: &str) -> OnvifResult<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(OnvifError::MissingField("EndpointReference".to_string()));
    }
    Ok(trimmed.to_string())
}
