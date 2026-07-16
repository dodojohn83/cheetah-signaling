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
}

#[cfg(test)]
mod checksum_tests {
    use crate::PluginError;
    use crate::checksum::verify_manifest_checksum;
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
    fn sha256_rejects_incorrect_digest() {
        assert!(verify_manifest_checksum(b"hello", "sha256", "deadbeef", &[]).is_err());
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
