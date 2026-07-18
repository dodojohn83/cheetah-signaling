//! Source record model consumed by the migration tool.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Entity kind in the source system.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    /// Unknown or unsupported entity.
    #[default]
    Unknown,
    /// Tenant / organisation.
    Tenant,
    /// Device.
    Device,
    /// Channel belonging to a device.
    Channel,
    /// GB/T 28181 platform / cascaded upper platform.
    Gb28181Platform,
    /// ONVIF endpoint.
    OnvifEndpoint,
    /// Secret reference that cannot be migrated as plaintext.
    SecretReference,
}

/// A single record exported from the old system.
///
/// The tool is intentionally schema-tolerant: only the fields needed to build
/// the target domain objects are required. Extra columns in CSV files are
/// ignored; extra keys in JSON objects are collected into `metadata`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct OldRecord {
    /// Entity kind.
    #[serde(default)]
    pub entity_type: EntityType,
    /// Tenant identifier in the old system.
    #[serde(default)]
    pub tenant_id: String,
    /// External/protocol identifier.
    #[serde(default)]
    pub external_id: String,
    /// Human-readable name.
    #[serde(default)]
    pub name: String,
    /// Protocol name, e.g. `gb28181` or `onvif`.
    #[serde(default)]
    pub protocol: String,
    /// Device kind, e.g. `camera` or `nvr`.
    #[serde(default)]
    pub kind: String,
    /// Authority / host / endpoint address.
    #[serde(default)]
    pub authority: String,
    /// For channels: external ID of the parent device.
    #[serde(default)]
    pub parent_device_id: String,
    /// For channels: channel kind, e.g. `video` or `ptz`.
    #[serde(default)]
    pub channel_kind: String,
    /// Whether the channel is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Free-form metadata and unknown JSON keys.
    #[serde(flatten)]
    pub metadata: BTreeMap<String, serde_json::Value>,
    /// Comma-separated list of credential fields that should be re-entered
    /// rather than migrated as plaintext.
    #[serde(default)]
    pub secret_fields: String,
}

impl OldRecord {
    /// Returns the parsed secret field names.
    pub fn secret_field_names(&self) -> Vec<&str> {
        self.secret_fields
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Returns true if this record carries a credential that must not be
    /// written to the target database as plaintext.
    pub fn has_secret(&self) -> bool {
        !self.secret_field_names().is_empty()
    }
}
