//! Profile registry, device matching and fixed-priority selection.

use std::collections::BTreeSet;

use super::profile::{CompatibilityProfile, ProfileMatchKey};
use super::{CompatibilityError, StandardVersion};

/// The priority at which a profile matches a device.
///
/// Ordered from least to most specific, so `Ord` selects the strongest match.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MatchSpecificity {
    /// Matches any device of the standard version.
    StandardGeneric,
    /// Matches on manufacturer only.
    Manufacturer,
    /// Matches on manufacturer and model.
    Model,
    /// Matches on manufacturer, model and firmware (most specific).
    Firmware,
}

/// A description of the device/platform being matched against the registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceDescriptor {
    standard_version: StandardVersion,
    manufacturer: Option<String>,
    model: Option<String>,
    firmware: Option<String>,
}

impl DeviceDescriptor {
    /// Creates a descriptor for the given standard version with no vendor
    /// details.
    pub fn new(standard_version: StandardVersion) -> Self {
        Self {
            standard_version,
            manufacturer: None,
            model: None,
            firmware: None,
        }
    }

    /// Sets the observed manufacturer.
    pub fn with_manufacturer(mut self, manufacturer: impl Into<String>) -> Self {
        self.manufacturer = Some(manufacturer.into());
        self
    }

    /// Sets the observed model.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Sets the observed firmware.
    pub fn with_firmware(mut self, firmware: impl Into<String>) -> Self {
        self.firmware = Some(firmware.into());
        self
    }

    /// The standard version reported by the device.
    pub fn standard_version(&self) -> StandardVersion {
        self.standard_version
    }
}

fn component_matches(key: Option<&str>, observed: Option<&str>) -> bool {
    match key {
        // A `None` key component is a wildcard.
        None => true,
        // A required key component only matches an observed value that equals it
        // (ASCII case-insensitive). A missing observed value never matches a
        // required component.
        Some(expected) => observed.is_some_and(|value| value.eq_ignore_ascii_case(expected)),
    }
}

fn key_matches(key: &ProfileMatchKey, device: &DeviceDescriptor) -> bool {
    component_matches(key.manufacturer(), device.manufacturer.as_deref())
        && component_matches(key.model(), device.model.as_deref())
        && component_matches(key.firmware(), device.firmware.as_deref())
}

/// The outcome of selecting a profile for a device.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileSelection<'a> {
    /// A profile matched at the given priority.
    Matched {
        /// The selected profile.
        profile: &'a CompatibilityProfile,
        /// The priority at which it matched.
        specificity: MatchSpecificity,
    },
    /// No configured profile matched the device.
    NoMatch,
}

/// A validated set of compatibility profiles with fixed-priority selection.
#[derive(Clone, Debug, Default)]
pub struct CompatibilityRegistry {
    profiles: Vec<CompatibilityProfile>,
}

impl CompatibilityRegistry {
    /// Builds a registry, validating every profile and rejecting duplicate ids
    /// or duplicate `(standard_version, match_key)` pairs that would make
    /// selection ambiguous.
    pub fn new(
        profiles: impl IntoIterator<Item = CompatibilityProfile>,
    ) -> Result<Self, CompatibilityError> {
        let profiles: Vec<CompatibilityProfile> = profiles.into_iter().collect();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();
        let mut seen_keys: BTreeSet<(StandardVersion, MatchKeyIdent)> = BTreeSet::new();
        for profile in &profiles {
            profile.validate()?;
            if !seen_ids.insert(profile.id().as_ref().to_string()) {
                return Err(CompatibilityError::DuplicateProfileId(
                    profile.id().to_string(),
                ));
            }
            let ident = MatchKeyIdent::from_key(profile.match_key());
            if !seen_keys.insert((profile.standard_version(), ident)) {
                return Err(CompatibilityError::DuplicateMatchKey {
                    standard: profile.standard_version(),
                });
            }
        }
        Ok(Self { profiles })
    }

    /// The number of profiles in the registry.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }

    /// Looks up a profile by id.
    pub fn get(&self, id: &str) -> Option<&CompatibilityProfile> {
        self.profiles.iter().find(|p| p.id().as_ref() == id)
    }

    /// Selects the best-matching profile for a device using the fixed priority
    /// exact firmware → model → manufacturer → standard generic.
    ///
    /// Returns [`CompatibilityError::AmbiguousMatch`] if two profiles match at
    /// the same top priority (this should already be prevented at construction,
    /// but is checked defensively).
    pub fn select(
        &self,
        device: &DeviceDescriptor,
    ) -> Result<ProfileSelection<'_>, CompatibilityError> {
        let mut best: Option<(&CompatibilityProfile, MatchSpecificity)> = None;
        let mut ambiguous = false;
        for profile in &self.profiles {
            if profile.standard_version() != device.standard_version() {
                continue;
            }
            if !key_matches(profile.match_key(), device) {
                continue;
            }
            let specificity = profile.match_key().specificity();
            match best {
                None => best = Some((profile, specificity)),
                Some((_, current)) if specificity > current => {
                    best = Some((profile, specificity));
                    ambiguous = false;
                }
                Some((_, current)) if specificity == current => ambiguous = true,
                Some(_) => {}
            }
        }
        match best {
            None => Ok(ProfileSelection::NoMatch),
            Some((_, specificity)) if ambiguous => {
                Err(CompatibilityError::AmbiguousMatch { specificity })
            }
            Some((profile, specificity)) => Ok(ProfileSelection::Matched {
                profile,
                specificity,
            }),
        }
    }
}

/// A comparable identity for a match key used only for duplicate detection.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
struct MatchKeyIdent {
    manufacturer: Option<String>,
    model: Option<String>,
    firmware: Option<String>,
}

impl MatchKeyIdent {
    fn from_key(key: &ProfileMatchKey) -> Self {
        // Lower-case ASCII so keys that only differ by case are treated as the
        // same, matching the case-insensitive selection semantics.
        Self {
            manufacturer: key.manufacturer().map(|s| s.to_ascii_lowercase()),
            model: key.model().map(|s| s.to_ascii_lowercase()),
            firmware: key.firmware().map(|s| s.to_ascii_lowercase()),
        }
    }
}
