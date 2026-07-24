//! Source record model consumed by the migration tool.

use crate::error::MigrationError;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Maximum byte length of a source tenant or external identifier.
const MAX_MIGRATION_ID_BYTES: usize = 256;
/// Maximum byte length of a display name.
const MAX_MIGRATION_NAME_BYTES: usize = 1024;
/// Maximum byte length of an authority / endpoint address.
const MAX_MIGRATION_AUTHORITY_BYTES: usize = 256;
/// Maximum byte length of a protocol, device kind or channel kind value.
const MAX_MIGRATION_TAG_BYTES: usize = 64;
/// Maximum byte length of the `secret_fields` comma-separated list.
const MAX_MIGRATION_SECRET_FIELDS_BYTES: usize = 1024;
/// Maximum number of distinct secret field names.
const MAX_MIGRATION_SECRET_FIELDS_COUNT: usize = 64;
/// Maximum number of metadata entries.
const MAX_MIGRATION_METADATA_KEYS: usize = 64;
/// Maximum byte length of a metadata key.
const MAX_MIGRATION_METADATA_KEY_BYTES: usize = 128;
/// Maximum byte length of a metadata string value.
const MAX_MIGRATION_METADATA_VALUE_BYTES: usize = 4096;

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
    /// Validates field sizes and metadata shape before the record is used to
    /// build stable identifiers or domain aggregates.
    pub(crate) fn validate(&self) -> Result<(), MigrationError> {
        if self.tenant_id.len() > MAX_MIGRATION_ID_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "tenant_id exceeds maximum length".to_string(),
            });
        }
        if self.external_id.len() > MAX_MIGRATION_ID_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "external_id exceeds maximum length".to_string(),
            });
        }
        if self.parent_device_id.len() > MAX_MIGRATION_ID_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "parent_device_id exceeds maximum length".to_string(),
            });
        }
        if self.name.len() > MAX_MIGRATION_NAME_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "name exceeds maximum length".to_string(),
            });
        }
        if self.authority.len() > MAX_MIGRATION_AUTHORITY_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "authority exceeds maximum length".to_string(),
            });
        }
        if self.protocol.len() > MAX_MIGRATION_TAG_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "protocol exceeds maximum length".to_string(),
            });
        }
        if self.kind.len() > MAX_MIGRATION_TAG_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "kind exceeds maximum length".to_string(),
            });
        }
        if self.channel_kind.len() > MAX_MIGRATION_TAG_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "channel_kind exceeds maximum length".to_string(),
            });
        }
        if self.secret_fields.len() > MAX_MIGRATION_SECRET_FIELDS_BYTES {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "secret_fields exceeds maximum length".to_string(),
            });
        }
        if self.secret_field_names().len() > MAX_MIGRATION_SECRET_FIELDS_COUNT {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "secret_fields contains too many entries".to_string(),
            });
        }
        if self.metadata.len() > MAX_MIGRATION_METADATA_KEYS {
            return Err(MigrationError::InvalidRecord {
                row: 0,
                message: "metadata contains too many entries".to_string(),
            });
        }
        for (key, value) in &self.metadata {
            if key.len() > MAX_MIGRATION_METADATA_KEY_BYTES {
                return Err(MigrationError::InvalidRecord {
                    row: 0,
                    message: "metadata key exceeds maximum length".to_string(),
                });
            }
            match value {
                Value::String(s) if s.len() <= MAX_MIGRATION_METADATA_VALUE_BYTES => {}
                Value::Number(_) | Value::Bool(_) => {}
                Value::Null => {}
                _ => {
                    return Err(MigrationError::InvalidRecord {
                        row: 0,
                        message: "metadata value exceeds maximum length or is not a scalar"
                            .to_string(),
                    });
                }
            }
        }
        Ok(())
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_record() -> OldRecord {
        OldRecord {
            entity_type: EntityType::Device,
            tenant_id: "tenant-a".to_string(),
            external_id: "cam-01".to_string(),
            name: "Camera 01".to_string(),
            protocol: "gb28181".to_string(),
            kind: "camera".to_string(),
            authority: "192.0.2.1:5060".to_string(),
            parent_device_id: String::new(),
            channel_kind: String::new(),
            enabled: true,
            metadata: BTreeMap::new(),
            secret_fields: String::new(),
        }
    }

    #[test]
    fn valid_record_passes() {
        assert!(valid_record().validate().is_ok());
    }

    #[test]
    fn rejects_oversized_external_id() {
        let mut r = valid_record();
        r.external_id = "x".repeat(MAX_MIGRATION_ID_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_tenant_id() {
        let mut r = valid_record();
        r.tenant_id = "x".repeat(MAX_MIGRATION_ID_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_parent_device_id() {
        let mut r = valid_record();
        r.entity_type = EntityType::Channel;
        r.parent_device_id = "x".repeat(MAX_MIGRATION_ID_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_name() {
        let mut r = valid_record();
        r.name = "x".repeat(MAX_MIGRATION_NAME_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_authority() {
        let mut r = valid_record();
        r.authority = "x".repeat(MAX_MIGRATION_AUTHORITY_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_secret_fields() {
        let mut r = valid_record();
        r.secret_fields = "x".repeat(MAX_MIGRATION_SECRET_FIELDS_BYTES + 1);
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_too_many_secret_field_names() {
        let mut r = valid_record();
        let names: Vec<String> = (0..=MAX_MIGRATION_SECRET_FIELDS_COUNT)
            .map(|i| format!("field{i}"))
            .collect();
        r.secret_fields = names.join(",");
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_too_many_metadata_keys() {
        let mut r = valid_record();
        for i in 0..=MAX_MIGRATION_METADATA_KEYS {
            r.metadata.insert(format!("key{i}"), json!("value"));
        }
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_oversized_metadata_value() {
        let mut r = valid_record();
        r.metadata.insert(
            "note".to_string(),
            json!("x".repeat(MAX_MIGRATION_METADATA_VALUE_BYTES + 1)),
        );
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_non_scalar_metadata_value() {
        let mut r = valid_record();
        r.metadata.insert("nested".to_string(), json!({ "a": 1 }));
        assert!(r.validate().is_err());
    }
}
