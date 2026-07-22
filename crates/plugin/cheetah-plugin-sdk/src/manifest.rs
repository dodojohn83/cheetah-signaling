//! Plugin manifest and capability declarations.

use crate::error::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

/// A human-readable plugin type identifier, e.g. `cheetah/gb28181`.
///
/// This is separate from the runtime [`PluginId`](cheetah_signal_types::PluginId)
/// so the same plugin binary can be instantiated multiple times with different
/// configuration instances.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PluginName(String);

impl PluginName {
    /// Creates a plugin name after validating the format.
    ///
    /// Valid names are non-empty, at most 128 bytes, and contain only
    /// lowercase ASCII letters, digits, hyphens, underscores and forward slashes.
    pub fn new(name: impl Into<String>) -> Result<Self, PluginError> {
        let name = name.into();
        if name.is_empty() {
            return Err(PluginError::InvalidManifest(
                "plugin name must not be empty".to_string(),
            ));
        }
        if name.len() > 128 {
            return Err(PluginError::InvalidManifest(
                "plugin name must not exceed 128 bytes".to_string(),
            ));
        }
        if !name.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_' || b == b'/'
        }) {
            return Err(PluginError::InvalidManifest(
                "plugin name contains invalid characters".to_string(),
            ));
        }
        Ok(Self(name))
    }

    /// Creates a `PluginName` from a trusted static string without re-validating.
    ///
    /// The caller is responsible for ensuring the string satisfies the
    /// `PluginName` validation rules. This is intended for static plugin
    /// names that are known correct at build time.
    pub fn from_static_unchecked(name: &'static str) -> Self {
        debug_assert!(
            !name.is_empty()
                && name.len() <= 128
                && name.bytes().all(|b| {
                    b.is_ascii_lowercase()
                        || b.is_ascii_digit()
                        || b == b'-'
                        || b == b'_'
                        || b == b'/'
                }),
            "static plugin name must be valid: {name}"
        );
        Self(name.to_string())
    }

    /// Returns the name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PluginName {
    fn default() -> Self {
        Self("unknown".to_string())
    }
}

impl AsRef<str> for PluginName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PluginName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for PluginName {
    type Err = PluginError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// A semantic version of a plugin or the SDK.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PluginVersion(String);

impl PluginVersion {
    /// Parses a plugin version.
    pub fn new(version: impl Into<String>) -> Result<Self, PluginError> {
        let version = version.into();
        if semver::Version::parse(&version).is_err() {
            return Err(PluginError::InvalidManifest(format!(
                "plugin version {version:?} is not valid semver"
            )));
        }
        Ok(Self(version))
    }

    /// Returns the version string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses the version as a [`semver::Version`].
    pub fn parse(&self) -> Result<semver::Version, PluginError> {
        semver::Version::parse(&self.0).map_err(|e| {
            PluginError::InvalidManifest(format!(
                "plugin version {version:?}: {e}",
                version = self.0
            ))
        })
    }
}

impl Default for PluginVersion {
    fn default() -> Self {
        Self("0.0.0".to_string())
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for PluginVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for PluginVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for PluginVersion {
    type Err = PluginError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// A semantic-version range for the SDK versions a plugin is compatible with.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SdkVersionReq(String);

impl SdkVersionReq {
    /// Parses an SDK version requirement.
    pub fn new(req: impl Into<String>) -> Result<Self, PluginError> {
        let req = req.into();
        if semver::VersionReq::parse(&req).is_err() {
            return Err(PluginError::InvalidManifest(format!(
                "SDK version requirement {req:?} is not valid"
            )));
        }
        Ok(Self(req))
    }

    /// Returns the requirement string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parses the requirement as a [`semver::VersionReq`].
    pub fn parse(&self) -> Result<semver::VersionReq, PluginError> {
        semver::VersionReq::parse(&self.0).map_err(|e| {
            PluginError::InvalidManifest(format!(
                "SDK version requirement {req:?}: {e}",
                req = self.0
            ))
        })
    }

    /// Returns whether the given host SDK version satisfies this requirement.
    pub fn matches(&self, host: &semver::Version) -> Result<bool, PluginError> {
        Ok(self.parse()?.matches(host))
    }
}

impl Default for SdkVersionReq {
    fn default() -> Self {
        Self("*".to_string())
    }
}

impl fmt::Display for SdkVersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for SdkVersionReq {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SdkVersionReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl std::str::FromStr for SdkVersionReq {
    type Err = PluginError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Direction of protocol traffic supported by the plugin.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolDirection {
    /// Device-initiated connections / registrations.
    #[default]
    Inbound,
    /// Platform-initiated connections to devices.
    Outbound,
    /// Both directions.
    Bidirectional,
}

/// A protocol supported by a plugin.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ProtocolCapability {
    /// Protocol name, e.g. `gb28181` or `onvif`.
    pub protocol: String,
    /// Traffic direction.
    pub direction: ProtocolDirection,
    /// Optional media transport this protocol can negotiate.
    pub media_transport: Option<String>,
}

/// A permission granted to a plugin by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginPermission {
    /// Publish protocol events to the message bus.
    PublishEvents,
    /// Read device configuration and credentials by reference.
    ReadDeviceConfig,
    /// Request media session allocation.
    RequestMediaSession,
    /// Register protocol endpoints (UDP/TCP ports, HTTP paths, etc.).
    RegisterEndpoint,
    /// Make outbound network requests.
    OutboundNetwork,
    /// Read tenant-scoped resources.
    ReadTenant,
}

/// Resource budget requested by a plugin instance.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ResourceBudget {
    /// Maximum resident memory in megabytes.
    pub max_memory_mb: u64,
    /// Maximum CPU in millicores.
    pub max_cpu_milli: u64,
    /// Maximum open file descriptors / sockets.
    pub max_fds: u64,
    /// Maximum outbound network bandwidth in megabits per second.
    pub max_bandwidth_mbps: u64,
}

/// JSON Schema describing the plugin's configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfigSchema {
    /// JSON Schema document.
    pub schema: serde_json::Value,
    /// Required top-level fields.
    #[serde(default)]
    pub required: Vec<String>,
}

/// Plugin entry point descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEntry {
    /// Built-in Rust driver identified by a fully qualified type/module path.
    BuiltIn {
        /// Rust module path or factory name.
        path: String,
    },
    /// Out-of-process executable path or package reference.
    OutOfProcess {
        /// Executable path or package name.
        path: String,
    },
}

/// Checksum / signature metadata for a plugin manifest.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct PluginChecksum {
    /// Algorithm name, e.g. `sha256` or `hmac-sha256`.
    pub algorithm: String,
    /// Hex-encoded digest.
    pub digest: String,
}

/// Declarative manifest for a protocol plugin.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Human-readable plugin type name.
    pub name: PluginName,
    /// Plugin version.
    pub version: PluginVersion,
    /// SDK compatibility range.
    pub sdk_version: SdkVersionReq,
    /// Protocols provided by this plugin.
    pub protocols: Vec<ProtocolCapability>,
    /// Plugin entry point.
    pub entry: PluginEntry,
    /// Permissions required by the plugin.
    pub permissions: Vec<PluginPermission>,
    /// Configuration schema.
    pub config_schema: ConfigSchema,
    /// Resource budget requested by the plugin.
    pub resource_budget: ResourceBudget,
    /// Optional checksum / signature for the manifest payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<PluginChecksum>,
    /// Free-form metadata.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl PluginManifest {
    /// Validates the manifest and returns the parsed semver version.
    pub fn validate(&self) -> Result<semver::Version, PluginError> {
        if self.protocols.is_empty() {
            return Err(PluginError::InvalidManifest(
                "manifest must declare at least one protocol".to_string(),
            ));
        }
        for protocol in &self.protocols {
            if protocol.protocol.is_empty() {
                return Err(PluginError::InvalidManifest(
                    "protocol name must not be empty".to_string(),
                ));
            }
        }
        if self.permissions.is_empty() {
            return Err(PluginError::InvalidManifest(
                "manifest must declare at least one permission".to_string(),
            ));
        }
        if self.config_schema.schema.is_null() {
            return Err(PluginError::InvalidManifest(
                "config schema must be a valid JSON object".to_string(),
            ));
        }
        self.version.parse()
    }
}
