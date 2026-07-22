//! GB28181 compatibility profile resolution.
//!
//! Profiles are loaded from configuration as a set of [`CompatibilityProfile`]
//! values and resolved against a [`ProfileSelector`] built from device facts.
//! The selected profile is pinned to the [`ProtocolSession`] at binding time,
//! so runtime profile changes do not alter in-flight dialogs.

use cheetah_domain::{CompatibilityProfile, ProfileSelector};

/// Error returned when a profile cannot be uniquely resolved.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProfileResolveError {
    /// More than one profile matched the selector with the same priority.
    #[error("ambiguous compatibility profile match for selector {selector:?}")]
    Ambiguous {
        /// The selector that produced the ambiguity.
        selector: ProfileSelector,
    },
}

/// Resolves the best matching [`CompatibilityProfile`] for a device.
#[derive(Clone, Debug, Default)]
pub struct ProfileResolver {
    profiles: Vec<CompatibilityProfile>,
}

impl ProfileResolver {
    /// Creates a resolver over the given profile set.
    pub fn new(profiles: Vec<CompatibilityProfile>) -> Self {
        Self { profiles }
    }

    /// Returns the profiles held by this resolver.
    pub fn profiles(&self) -> &[CompatibilityProfile] {
        &self.profiles
    }

    /// Resolves the best matching profile for `selector`.
    ///
    /// Priority follows exact firmware → model → manufacturer → standard
    /// version → default. If two profiles tie at the same highest priority, the
    /// call returns [`ProfileResolveError::Ambiguous`].
    pub fn resolve(
        &self,
        selector: &ProfileSelector,
    ) -> Result<CompatibilityProfile, ProfileResolveError> {
        let scores: Vec<u32> = self.profiles.iter().map(|p| p.score(selector)).collect();
        let best_score = scores.iter().copied().filter(|s| *s > 0).max().unwrap_or(0);
        if best_score == 0 {
            return Ok(CompatibilityProfile::default());
        }

        let mut best: Option<&CompatibilityProfile> = None;
        for (profile, score) in self.profiles.iter().zip(scores) {
            if score != best_score {
                continue;
            }
            if best.is_some() {
                return Err(ProfileResolveError::Ambiguous {
                    selector: selector.clone(),
                });
            }
            best = Some(profile);
        }

        Ok(best.cloned().unwrap_or_default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use cheetah_domain::{CompatibilityCapability, CompatibilityProfile};

    fn profile(
        id: &str,
        standard_version: Option<&str>,
        manufacturer: Option<&str>,
        model: Option<&str>,
        firmware: Option<&str>,
        capabilities: &[CompatibilityCapability],
    ) -> CompatibilityProfile {
        CompatibilityProfile {
            profile_id: Some(id.to_string()),
            standard_version: standard_version.map(|s| s.to_string()),
            manufacturer: manufacturer.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            firmware: firmware.map(|s| s.to_string()),
            capabilities: capabilities.to_vec(),
            evidence_ref: Some(format!("testdata/gb28181/profiles/{id}.meta.toml")),
            revision: 1,
        }
    }

    fn selector(
        standard_version: Option<&str>,
        manufacturer: Option<&str>,
        model: Option<&str>,
        firmware: Option<&str>,
    ) -> ProfileSelector {
        ProfileSelector {
            standard_version: standard_version.map(|s| s.to_string()),
            manufacturer: manufacturer.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            firmware: firmware.map(|s| s.to_string()),
        }
    }

    #[test]
    fn exact_firmware_wins_over_model() {
        let resolver = ProfileResolver::new(vec![
            profile(
                "generic-2016",
                Some("2016"),
                None,
                None,
                None,
                &[CompatibilityCapability::Gb2016],
            ),
            profile(
                "vendor-model",
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                None,
                &[
                    CompatibilityCapability::Gb2016,
                    CompatibilityCapability::Broadcast,
                ],
            ),
            profile(
                "vendor-model-fw",
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                Some("1.2.3"),
                &[CompatibilityCapability::PresetQuery],
            ),
        ]);

        let selected = resolver
            .resolve(&selector(
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                Some("1.2.3"),
            ))
            .unwrap();
        assert_eq!(selected.profile_id.as_deref(), Some("vendor-model-fw"));
        assert!(selected.has(CompatibilityCapability::PresetQuery));
    }

    #[test]
    fn manufacturer_fallback_when_model_unspecified() {
        let resolver = ProfileResolver::new(vec![
            profile(
                "vendor",
                Some("2016"),
                Some("Vendor"),
                None,
                None,
                &[CompatibilityCapability::CatalogNotify],
            ),
            profile(
                "generic",
                Some("2016"),
                None,
                None,
                None,
                &[CompatibilityCapability::Gb2016],
            ),
        ]);

        let selected = resolver
            .resolve(&selector(Some("2016"), Some("Vendor"), None, None))
            .unwrap();
        assert_eq!(selected.profile_id.as_deref(), Some("vendor"));
    }

    #[test]
    fn broad_standard_fallback_matches_devices_with_extra_details() {
        let resolver = ProfileResolver::new(vec![profile(
            "generic-2016",
            Some("2016"),
            None,
            None,
            None,
            &[CompatibilityCapability::Gb2016],
        )]);

        // A device that reports manufacturer, model and firmware but has no
        // more specific profile must fall back to the standard-only profile,
        // not the default empty profile.
        let selected = resolver
            .resolve(&selector(
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                Some("1.2.3"),
            ))
            .unwrap();
        assert_eq!(selected.profile_id.as_deref(), Some("generic-2016"));
    }

    #[test]
    fn model_only_outranks_manufacturer_plus_standard() {
        let resolver = ProfileResolver::new(vec![
            profile(
                "model-specific",
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                None,
                &[CompatibilityCapability::PresetQuery],
            ),
            profile(
                "vendor-all",
                Some("2016"),
                Some("Vendor"),
                None,
                None,
                &[CompatibilityCapability::CatalogNotify],
            ),
        ]);

        let selected = resolver
            .resolve(&selector(
                Some("2016"),
                Some("Vendor"),
                Some("Model-X"),
                None,
            ))
            .unwrap();
        assert_eq!(selected.profile_id.as_deref(), Some("model-specific"));
    }

    #[test]
    fn default_used_when_nothing_matches() {
        let resolver = ProfileResolver::new(vec![]);
        let selected = resolver
            .resolve(&selector(Some("2016"), Some("Vendor"), None, None))
            .unwrap();
        assert_eq!(selected.profile_id, None);
        assert!(selected.capabilities.is_empty());
    }

    #[test]
    fn ambiguous_match_returns_error() {
        let resolver = ProfileResolver::new(vec![
            profile(
                "a",
                Some("2016"),
                Some("Vendor"),
                None,
                None,
                &[CompatibilityCapability::CatalogNotify],
            ),
            profile(
                "b",
                Some("2016"),
                Some("Vendor"),
                None,
                None,
                &[CompatibilityCapability::AlarmSubscription],
            ),
        ]);

        assert!(
            resolver
                .resolve(&selector(Some("2016"), Some("Vendor"), None, None))
                .is_err()
        );
    }
}
