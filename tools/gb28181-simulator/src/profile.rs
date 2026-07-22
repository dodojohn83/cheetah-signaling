//! Device profiles.
//!
//! Profiles separate the generic/standard behaviour from synthetic vendor
//! fixtures.  Synthetic vendor names (`vendor-a`, `vendor-b`) are behavioural
//! placeholders only; they are **not** interoperability evidence and never
//! claim conformance for any real manufacturer.

use crate::scenario::Profile;

/// A profile resolved into the concrete values a device needs.
#[derive(Clone, Debug)]
pub struct ResolvedProfile {
    /// Profile identifier.
    pub id: String,
    /// GB/T standard label.
    pub standard: String,
    /// Keepalive interval in milliseconds.
    pub keepalive_ms: u64,
    /// Number of catalog channels per device.
    pub catalog_items: u32,
    /// Synthetic manufacturer label (never a real vendor claim).
    pub manufacturer: String,
    /// Synthetic model label.
    pub model: String,
    /// Whether this profile is a synthetic vendor fixture.
    pub synthetic_vendor: bool,
}

impl ResolvedProfile {
    /// Resolves a scenario [`Profile`] into concrete device parameters.
    pub fn resolve(profile: &Profile) -> Self {
        let (manufacturer, model) = synthetic_identity(&profile.id);
        Self {
            id: profile.id.clone(),
            standard: profile.standard.clone(),
            keepalive_ms: profile.keepalive_ms,
            catalog_items: profile.catalog_items.max(1),
            manufacturer,
            model,
            synthetic_vendor: profile.synthetic_vendor,
        }
    }
}

fn synthetic_identity(id: &str) -> (String, String) {
    match id {
        "generic" => ("Cheetah".to_string(), "SIM-GENERIC".to_string()),
        other => (
            format!("Synthetic-{other}"),
            format!("SIM-{}", other.to_ascii_uppercase()),
        ),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn generic_profile_is_not_a_vendor() {
        let resolved = ResolvedProfile::resolve(&Profile::default());
        assert_eq!(resolved.manufacturer, "Cheetah");
        assert!(!resolved.synthetic_vendor);
    }

    #[test]
    fn vendor_profile_is_marked_synthetic() {
        let profile = Profile {
            id: "vendor-a".to_string(),
            synthetic_vendor: true,
            ..Profile::default()
        };
        let resolved = ResolvedProfile::resolve(&profile);
        assert!(resolved.manufacturer.starts_with("Synthetic-"));
        assert!(resolved.synthetic_vendor);
    }
}
