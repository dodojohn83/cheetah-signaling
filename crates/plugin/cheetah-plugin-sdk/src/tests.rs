//! Unit tests for the plugin SDK.

#[cfg(test)]
mod manifest_tests {
    use crate::PluginError;
    use crate::manifest::{
        ConfigSchema, PluginEntry, PluginManifest, PluginName, PluginPermission, PluginVersion,
        ProtocolCapability, ProtocolDirection, ResourceBudget, SdkVersionReq,
    };
    use crate::version::negotiate_sdk_version;
    use serde_json::json;

    fn valid_manifest() -> Result<PluginManifest, PluginError> {
        Ok(PluginManifest {
            name: PluginName::new("cheetah/test")?,
            version: PluginVersion::new("0.1.0")?,
            sdk_version: SdkVersionReq::new(">=1.0.0, <2.0.0")?,
            protocols: vec![ProtocolCapability {
                protocol: "test".to_string(),
                direction: ProtocolDirection::Bidirectional,
                media_transport: None,
            }],
            entry: PluginEntry::BuiltIn {
                path: "test".to_string(),
            },
            permissions: vec![PluginPermission::PublishEvents],
            config_schema: ConfigSchema {
                schema: json!({"type": "object"}),
                required: vec![],
            },
            resource_budget: ResourceBudget::default(),
            checksum: None,
            metadata: Default::default(),
        })
    }

    #[test]
    fn plugin_name_rejects_invalid_characters() {
        assert!(PluginName::new("UPPER").is_err());
        assert!(PluginName::new("space here").is_err());
        assert!(PluginName::new("").is_err());
    }

    #[test]
    fn plugin_version_rejects_non_semver() {
        assert!(PluginVersion::new("not-a-version").is_err());
    }

    #[test]
    fn sdk_version_req_rejects_invalid_range() {
        assert!(SdkVersionReq::new("broken").is_err());
    }

    #[test]
    fn manifest_validate_requires_protocols() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.protocols = vec![];
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_requires_permissions() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.permissions = vec![];
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_requires_config_schema() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.config_schema.schema = serde_json::Value::Null;
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn negotiation_succeeds_when_host_in_range() -> Result<(), PluginError> {
        let req = SdkVersionReq::new(">=1.0.0, <2.0.0")?;
        let host = semver::Version::new(1, 5, 0);
        assert_eq!(negotiate_sdk_version(&req, &host)?, host);
        Ok(())
    }

    #[test]
    fn negotiation_fails_when_host_out_of_range() -> Result<(), PluginError> {
        let req = SdkVersionReq::new(">=1.0.0, <2.0.0")?;
        let host = semver::Version::new(2, 0, 0);
        assert!(negotiate_sdk_version(&req, &host).is_err());
        Ok(())
    }

    #[test]
    fn plugin_version_rejects_oversized_value() {
        assert!(PluginVersion::new("1.".repeat(65)).is_err());
    }

    #[test]
    fn sdk_version_req_rejects_oversized_value() {
        assert!(SdkVersionReq::new(">=".repeat(65)).is_err());
    }

    #[test]
    fn manifest_validate_rejects_too_many_protocols() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.protocols = (0..33)
            .map(|i| ProtocolCapability {
                protocol: format!("proto-{i}"),
                direction: ProtocolDirection::Bidirectional,
                media_transport: None,
            })
            .collect();
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_protocol_name() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.protocols[0].protocol = "x".repeat(65);
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_media_transport() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.protocols[0].media_transport = Some("x".repeat(65));
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_too_many_permissions() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.permissions = (0..33).map(|_| PluginPermission::PublishEvents).collect();
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_entry_path() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.entry = PluginEntry::BuiltIn {
            path: "x".repeat(1025),
        };
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_checksum() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.checksum = Some(crate::manifest::PluginChecksum {
            algorithm: "sha256".to_string(),
            digest: "x".repeat(257),
        });
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_too_many_metadata_keys() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.metadata = (0..65)
            .map(|i| (format!("key-{i}"), "value".to_string()))
            .collect();
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_metadata() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.metadata = std::collections::HashMap::from([
            ("x".repeat(129), "value".to_string()),
            ("key".to_string(), "x".repeat(1025)),
        ]);
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_too_many_required_fields() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.config_schema.required = (0..129).map(|i| format!("field-{i}")).collect();
        assert!(manifest.validate().is_err());
        Ok(())
    }

    #[test]
    fn manifest_validate_rejects_oversized_required_field() -> Result<(), PluginError> {
        let mut manifest = valid_manifest()?;
        manifest.config_schema.required = vec!["x".repeat(129)];
        assert!(manifest.validate().is_err());
        Ok(())
    }
}

#[cfg(test)]
mod checksum_tests {
    use crate::PluginError;
    use crate::checksum::{MAX_ALGORITHM_BYTES, MAX_DIGEST_HEX_BYTES, verify_manifest_checksum};
    use hmac::Mac;
    use sha2::{Digest, Sha256};

    #[test]
    fn sha256_verifies_correct_digest() -> Result<(), PluginError> {
        let payload = b"hello manifest";
        let digest = hex::encode(Sha256::digest(payload));
        verify_manifest_checksum(payload, "sha256", &digest, &[])?;
        Ok(())
    }

    #[test]
    fn sha256_accepts_uppercase_digest() -> Result<(), PluginError> {
        let payload = b"hello manifest";
        let digest = hex::encode(Sha256::digest(payload)).to_ascii_uppercase();
        verify_manifest_checksum(payload, "sha256", &digest, &[])?;
        Ok(())
    }

    #[test]
    fn sha256_rejects_incorrect_digest() {
        assert!(verify_manifest_checksum(b"hello", "sha256", "deadbeef", &[]).is_err());
    }

    #[test]
    fn rejects_oversized_algorithm() {
        let algorithm = "a".repeat(MAX_ALGORITHM_BYTES + 1);
        assert!(verify_manifest_checksum(b"x", &algorithm, "a", &[]).is_err());
    }

    #[test]
    fn rejects_oversized_digest() {
        let digest = "a".repeat(MAX_DIGEST_HEX_BYTES + 1);
        assert!(matches!(
            verify_manifest_checksum(b"x", "sha256", &digest, &[]),
            Err(PluginError::InvalidChecksum)
        ));
    }

    #[test]
    fn hmac_sha256_verifies_with_secret() -> Result<(), PluginError> {
        let payload = b"hello manifest";
        let mut mac = hmac::Hmac::<Sha256>::new_from_slice(b"secret")
            .map_err(|e| PluginError::InvalidManifest(format!("invalid hmac key: {e}")))?;
        mac.update(payload);
        let digest = hex::encode(mac.finalize().into_bytes());
        verify_manifest_checksum(payload, "hmac-sha256", &digest, b"secret")?;
        Ok(())
    }
}

#[cfg(test)]
mod deserialization_tests {
    use crate::manifest::{
        ConfigSchema, PluginEntry, PluginManifest, PluginPermission, ProtocolCapability,
        ProtocolDirection, ResourceBudget,
    };
    use serde_json::json;

    #[test]
    fn manifest_round_trips_and_validates() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = PluginManifest {
            name: "cheetah/test".parse()?,
            version: "0.1.0".parse()?,
            sdk_version: ">=1.0.0, <2.0.0".parse()?,
            protocols: vec![ProtocolCapability {
                protocol: "test".to_string(),
                direction: ProtocolDirection::Bidirectional,
                media_transport: None,
            }],
            entry: PluginEntry::BuiltIn {
                path: "test".to_string(),
            },
            permissions: vec![PluginPermission::PublishEvents],
            config_schema: ConfigSchema {
                schema: json!({"type": "object"}),
                required: vec![],
            },
            resource_budget: ResourceBudget::default(),
            checksum: None,
            metadata: Default::default(),
        };

        let json = serde_json::to_string(&manifest)?;
        let decoded: PluginManifest = serde_json::from_str(&json)?;
        assert!(decoded.validate().is_ok());
        Ok(())
    }

    #[test]
    fn manifest_deserialization_rejects_invalid_version() {
        let json = json!({
            "name": "cheetah/test",
            "version": "not-a-version",
            "sdk_version": ">=1.0.0, <2.0.0",
            "protocols": [{"protocol": "test", "direction": "bidirectional"}],
            "entry": {"built_in": {"path": "test"}},
            "permissions": ["publish_events"],
            "config_schema": {"schema": {"type": "object"}, "required": []},
            "resource_budget": {
                "max_memory_mb": 0,
                "max_cpu_milli": 0,
                "max_fds": 0,
                "max_bandwidth_mbps": 0
            },
            "metadata": {}
        });
        assert!(serde_json::from_str::<PluginManifest>(&json.to_string()).is_err());
    }

    #[test]
    fn manifest_deserialization_rejects_invalid_name() {
        let json = json!({
            "name": "HAS UPPERCASE",
            "version": "0.1.0",
            "sdk_version": ">=1.0.0, <2.0.0",
            "protocols": [{"protocol": "test", "direction": "bidirectional"}],
            "entry": {"built_in": {"path": "test"}},
            "permissions": ["publish_events"],
            "config_schema": {"schema": {"type": "object"}, "required": []},
            "resource_budget": {
                "max_memory_mb": 0,
                "max_cpu_milli": 0,
                "max_fds": 0,
                "max_bandwidth_mbps": 0
            },
            "metadata": {}
        });
        assert!(serde_json::from_str::<PluginManifest>(&json.to_string()).is_err());
    }
}
