//! Manifest loading, validation and version negotiation for the plugin host.

use crate::error::PluginHostError;
use cheetah_plugin_sdk::{PluginManifest, negotiate_sdk_version, verify_manifest_checksum};
use std::fmt;

/// A manifest that has been validated against the host SDK version and checksum.
#[derive(Clone, Debug)]
pub struct ValidatedManifest {
    /// Original manifest.
    pub manifest: PluginManifest,
    /// Negotiated SDK version.
    pub sdk_version: semver::Version,
    /// Parsed plugin version.
    pub plugin_version: semver::Version,
}

/// Loads and validates a plugin manifest before activation.
pub struct ManifestLoader {
    host_sdk_version: semver::Version,
}

impl ManifestLoader {
    /// Creates a loader for the given host SDK version.
    pub fn new(host_sdk_version: semver::Version) -> Self {
        Self { host_sdk_version }
    }

    /// Validates a manifest and negotiates the SDK version.
    ///
    /// `payload` is the raw manifest bytes used for checksum verification.
    pub fn validate(
        &self,
        manifest: &PluginManifest,
        payload: &[u8],
    ) -> Result<ValidatedManifest, PluginHostError> {
        if let Some(checksum) = &manifest.checksum {
            if checksum.algorithm == "hmac-sha256" {
                return Err(PluginHostError::InvalidManifest(
                    "hmac-sha256 manifest checksum requires a secret and is not supported by the built-in loader".to_string(),
                ));
            }
            verify_manifest_checksum(payload, &checksum.algorithm, &checksum.digest, &[])?;
        }

        let plugin_version = manifest.validate()?;
        let sdk_version = negotiate_sdk_version(&manifest.sdk_version, &self.host_sdk_version)?;

        Ok(ValidatedManifest {
            manifest: manifest.clone(),
            sdk_version,
            plugin_version,
        })
    }
}

impl Default for ManifestLoader {
    fn default() -> Self {
        Self::new(semver::Version::new(1, 0, 0))
    }
}

impl fmt::Debug for ManifestLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManifestLoader")
            .field("host_sdk_version", &self.host_sdk_version)
            .finish()
    }
}
