//! The [`CompatibilityProfile`] aggregate, its configuration form, validation
//! and revision pinning.

use std::collections::BTreeSet;

use serde::Deserialize;

use super::overrides::{
    CatalogOverrides, CharsetPreference, DigestAlgorithmPreference, EndpointBehavior,
    MediaStatusOverrides, RportPolicy, SdpOverrides, SdpSetupPreference, SourceRoutePolicy,
};
use super::registry::MatchSpecificity;
use super::{CompatibilityCapability, CompatibilityError, StandardVersion};

/// A validated compatibility profile identifier.
///
/// Accepts 1-128 characters from `[A-Za-z0-9._:-]`, which covers ids such as
/// `"vendor-model-firmware"`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProfileId(String);

impl ProfileId {
    /// Maximum identifier length.
    pub const MAX_LEN: usize = 128;

    /// Creates a profile id, returning an error if it is empty, too long, or
    /// contains characters outside `[A-Za-z0-9._:-]`.
    pub fn new(id: impl AsRef<str>) -> Result<Self, CompatibilityError> {
        let id = id.as_ref();
        if id.is_empty() || id.len() > Self::MAX_LEN {
            return Err(CompatibilityError::InvalidProfileId);
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
        {
            return Err(CompatibilityError::InvalidProfileId);
        }
        Ok(Self(id.to_string()))
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ProfileId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A monotonic profile revision used to pin runtime sessions.
///
/// A running dialog captures the revision that was active when it started so a
/// later hot reload cannot silently change its semantics.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProfileRevision(u32);

impl ProfileRevision {
    /// The initial revision assigned to a freshly loaded profile.
    pub const INITIAL: ProfileRevision = ProfileRevision(1);

    /// Creates a revision from a raw counter. Zero is rejected because a valid
    /// profile always has at least the initial revision.
    pub fn new(value: u32) -> Result<Self, CompatibilityError> {
        if value == 0 {
            return Err(CompatibilityError::InvalidMatchKey("revision must be >= 1"));
        }
        Ok(Self(value))
    }

    /// Returns the raw revision counter.
    pub fn get(self) -> u32 {
        self.0
    }

    /// Returns the next revision, saturating at `u32::MAX`.
    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl std::fmt::Display for ProfileRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The manufacturer/model/firmware match key of a profile.
///
/// The three components form a hierarchy: a `firmware` match requires a `model`,
/// and a `model` match requires a `manufacturer`. An all-`None` key is the
/// standard-generic fallback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileMatchKey {
    manufacturer: Option<String>,
    model: Option<String>,
    firmware: Option<String>,
}

impl ProfileMatchKey {
    /// Maximum length of any single match-key component.
    pub const MAX_COMPONENT_LEN: usize = 128;

    /// Creates and validates a match key.
    pub fn new(
        manufacturer: Option<String>,
        model: Option<String>,
        firmware: Option<String>,
    ) -> Result<Self, CompatibilityError> {
        let manufacturer = normalize_component(manufacturer, "manufacturer")?;
        let model = normalize_component(model, "model")?;
        let firmware = normalize_component(firmware, "firmware")?;
        if firmware.is_some() && model.is_none() {
            return Err(CompatibilityError::InvalidMatchKey(
                "firmware match requires a model",
            ));
        }
        if model.is_some() && manufacturer.is_none() {
            return Err(CompatibilityError::InvalidMatchKey(
                "model match requires a manufacturer",
            ));
        }
        Ok(Self {
            manufacturer,
            model,
            firmware,
        })
    }

    /// The standard-generic key (matches any device of the standard version).
    pub fn generic() -> Self {
        Self {
            manufacturer: None,
            model: None,
            firmware: None,
        }
    }

    /// The manufacturer component, if any.
    pub fn manufacturer(&self) -> Option<&str> {
        self.manufacturer.as_deref()
    }

    /// The model component, if any.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// The firmware component, if any.
    pub fn firmware(&self) -> Option<&str> {
        self.firmware.as_deref()
    }

    /// The priority at which this key participates in selection.
    pub fn specificity(&self) -> MatchSpecificity {
        if self.firmware.is_some() {
            MatchSpecificity::Firmware
        } else if self.model.is_some() {
            MatchSpecificity::Model
        } else if self.manufacturer.is_some() {
            MatchSpecificity::Manufacturer
        } else {
            MatchSpecificity::StandardGeneric
        }
    }
}

fn normalize_component(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<String>, CompatibilityError> {
    match value {
        None => Ok(None),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(CompatibilityError::InvalidMatchKey(field));
            }
            if trimmed.len() > ProfileMatchKey::MAX_COMPONENT_LEN
                || trimmed.chars().any(|c| c.is_control())
            {
                return Err(CompatibilityError::InvalidMatchKey(field));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// An explicit, auditable GB28181 compatibility profile.
///
/// Fields are private so a profile can only be built through the validating
/// [`CompatibilityProfile::from_config`] or [`CompatibilityProfile::builder`]
/// entry points, which enforce every invariant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityProfile {
    id: ProfileId,
    revision: ProfileRevision,
    standard_version: StandardVersion,
    match_key: ProfileMatchKey,
    digest: DigestAlgorithmPreference,
    charset: CharsetPreference,
    endpoint: EndpointBehavior,
    catalog: CatalogOverrides,
    sdp: SdpOverrides,
    media_status: MediaStatusOverrides,
    capabilities: BTreeSet<CompatibilityCapability>,
    evidence_ref: Option<String>,
}

impl CompatibilityProfile {
    /// Starts building a profile with all overrides at their safe defaults.
    pub fn builder(
        id: ProfileId,
        standard_version: StandardVersion,
        match_key: ProfileMatchKey,
    ) -> CompatibilityProfileBuilder {
        CompatibilityProfileBuilder {
            id,
            revision: ProfileRevision::INITIAL,
            standard_version,
            match_key,
            digest: DigestAlgorithmPreference::Sha256,
            charset: CharsetPreference::Utf8Strict,
            endpoint: EndpointBehavior::default(),
            catalog: CatalogOverrides::default(),
            sdp: SdpOverrides::default(),
            media_status: MediaStatusOverrides::default(),
            capabilities: BTreeSet::new(),
            evidence_ref: None,
        }
    }

    /// Builds a validated profile from its deserialized configuration form.
    pub fn from_config(config: CompatibilityProfileConfig) -> Result<Self, CompatibilityError> {
        let id = ProfileId::new(&config.id)?;
        let revision = ProfileRevision::new(config.revision)?;
        let standard_version = StandardVersion::parse(&config.standard_version).ok_or(
            CompatibilityError::UnknownStandardVersion(config.standard_version.clone()),
        )?;
        let match_key = ProfileMatchKey::new(config.manufacturer, config.model, config.firmware)?;

        let digest = match config.digest_algorithm.as_deref() {
            None => DigestAlgorithmPreference::Sha256,
            Some(value) => DigestAlgorithmPreference::parse(value).ok_or_else(|| {
                CompatibilityError::UnknownOverrideValue {
                    field: "digest_algorithm",
                    value: value.to_string(),
                }
            })?,
        };
        let charset = match config.charset.as_deref() {
            None => CharsetPreference::Utf8Strict,
            Some(value) => CharsetPreference::parse(value).ok_or_else(|| {
                CompatibilityError::UnknownOverrideValue {
                    field: "charset",
                    value: value.to_string(),
                }
            })?,
        };
        let rport = match config.rport.as_deref() {
            None => RportPolicy::Honor,
            Some(value) => RportPolicy::parse(value).ok_or_else(|| {
                CompatibilityError::UnknownOverrideValue {
                    field: "rport",
                    value: value.to_string(),
                }
            })?,
        };
        let source_route = match config.source_route.as_deref() {
            None => SourceRoutePolicy::DialogTarget,
            Some(value) => SourceRoutePolicy::parse(value).ok_or_else(|| {
                CompatibilityError::UnknownOverrideValue {
                    field: "source_route",
                    value: value.to_string(),
                }
            })?,
        };
        let endpoint = EndpointBehavior::new(rport, source_route);
        let catalog = match config.catalog_fragment_size {
            None => CatalogOverrides::default(),
            Some(size) => CatalogOverrides::new(size)?,
        };
        let sdp_setup = match config.sdp_setup.as_deref() {
            None => SdpSetupPreference::Negotiated,
            Some(value) => SdpSetupPreference::parse(value).ok_or_else(|| {
                CompatibilityError::UnknownOverrideValue {
                    field: "sdp_setup",
                    value: value.to_string(),
                }
            })?,
        };
        let sdp = SdpOverrides::new(
            config.sdp_emit_ssrc.unwrap_or(true),
            sdp_setup,
            config.sdp_send_media_before_ack.unwrap_or(false),
        );
        let media_status = MediaStatusOverrides::new(
            config.media_status_accept_121.unwrap_or(false),
            config.broadcast_handshake.unwrap_or(false),
        );

        let mut capabilities = BTreeSet::new();
        for token in &config.capabilities {
            let capability = CompatibilityCapability::parse(token)
                .ok_or_else(|| CompatibilityError::UnknownCapability(token.clone()))?;
            capabilities.insert(capability);
        }

        let evidence_ref = normalize_component(config.evidence_ref, "evidence_ref")?;

        let profile = Self {
            id,
            revision,
            standard_version,
            match_key,
            digest,
            charset,
            endpoint,
            catalog,
            sdp,
            media_status,
            capabilities,
            evidence_ref,
        };
        profile.validate()?;
        Ok(profile)
    }

    /// Re-validates the profile's cross-field invariants.
    ///
    /// Individual fields are validated at construction; this checks the
    /// relationships between overrides and declared capabilities so an enabled
    /// behavior always has a matching, auditable capability.
    pub fn validate(&self) -> Result<(), CompatibilityError> {
        if self.media_status.broadcast_handshake()
            && !self.supports(CompatibilityCapability::Broadcast)
        {
            return Err(CompatibilityError::InvalidMatchKey(
                "broadcast_handshake requires the broadcast capability",
            ));
        }
        if self.media_status.accept_status_code_121()
            && !self.supports(CompatibilityCapability::MediaStatusReport)
        {
            return Err(CompatibilityError::InvalidMatchKey(
                "media_status_accept_121 requires the media_status_report capability",
            ));
        }
        Ok(())
    }

    /// The profile identifier.
    pub fn id(&self) -> &ProfileId {
        &self.id
    }

    /// The profile revision.
    pub fn revision(&self) -> ProfileRevision {
        self.revision
    }

    /// The standard version this profile applies to.
    pub fn standard_version(&self) -> StandardVersion {
        self.standard_version
    }

    /// The match key.
    pub fn match_key(&self) -> &ProfileMatchKey {
        &self.match_key
    }

    /// The digest algorithm preference.
    pub fn digest(&self) -> DigestAlgorithmPreference {
        self.digest
    }

    /// The charset preference.
    pub fn charset(&self) -> CharsetPreference {
        self.charset
    }

    /// The endpoint routing behavior.
    pub fn endpoint(&self) -> &EndpointBehavior {
        &self.endpoint
    }

    /// The catalog overrides.
    pub fn catalog(&self) -> &CatalogOverrides {
        &self.catalog
    }

    /// The SDP overrides.
    pub fn sdp(&self) -> &SdpOverrides {
        &self.sdp
    }

    /// The MediaStatus overrides.
    pub fn media_status(&self) -> &MediaStatusOverrides {
        &self.media_status
    }

    /// The provenance/evidence reference for this profile's fixtures, if any.
    pub fn evidence_ref(&self) -> Option<&str> {
        self.evidence_ref.as_deref()
    }

    /// Whether the profile declares the given capability.
    pub fn supports(&self, capability: CompatibilityCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// The set of declared capabilities.
    pub fn capabilities(&self) -> &BTreeSet<CompatibilityCapability> {
        &self.capabilities
    }

    /// Negotiates the effective capabilities for a peer by intersecting the
    /// profile's declared capabilities with those the peer requests. A profile
    /// never grants a capability it did not declare, regardless of what the peer
    /// asks for.
    pub fn negotiate<I>(&self, requested: I) -> BTreeSet<CompatibilityCapability>
    where
        I: IntoIterator<Item = CompatibilityCapability>,
    {
        requested
            .into_iter()
            .filter(|cap| self.capabilities.contains(cap))
            .collect()
    }

    /// Pins the current profile revision into an immutable snapshot for a
    /// running session.
    pub fn pin(&self) -> PinnedProfile {
        PinnedProfile {
            profile: self.clone(),
        }
    }
}

/// A builder for [`CompatibilityProfile`] used by tests and programmatic setup.
#[derive(Clone, Debug)]
pub struct CompatibilityProfileBuilder {
    id: ProfileId,
    revision: ProfileRevision,
    standard_version: StandardVersion,
    match_key: ProfileMatchKey,
    digest: DigestAlgorithmPreference,
    charset: CharsetPreference,
    endpoint: EndpointBehavior,
    catalog: CatalogOverrides,
    sdp: SdpOverrides,
    media_status: MediaStatusOverrides,
    capabilities: BTreeSet<CompatibilityCapability>,
    evidence_ref: Option<String>,
}

impl CompatibilityProfileBuilder {
    /// Sets the profile revision.
    pub fn revision(mut self, revision: ProfileRevision) -> Self {
        self.revision = revision;
        self
    }

    /// Sets the digest algorithm preference.
    pub fn digest(mut self, digest: DigestAlgorithmPreference) -> Self {
        self.digest = digest;
        self
    }

    /// Sets the charset preference.
    pub fn charset(mut self, charset: CharsetPreference) -> Self {
        self.charset = charset;
        self
    }

    /// Sets the endpoint behavior.
    pub fn endpoint(mut self, endpoint: EndpointBehavior) -> Self {
        self.endpoint = endpoint;
        self
    }

    /// Sets the catalog overrides.
    pub fn catalog(mut self, catalog: CatalogOverrides) -> Self {
        self.catalog = catalog;
        self
    }

    /// Sets the SDP overrides.
    pub fn sdp(mut self, sdp: SdpOverrides) -> Self {
        self.sdp = sdp;
        self
    }

    /// Sets the MediaStatus overrides.
    pub fn media_status(mut self, media_status: MediaStatusOverrides) -> Self {
        self.media_status = media_status;
        self
    }

    /// Adds a declared capability.
    pub fn capability(mut self, capability: CompatibilityCapability) -> Self {
        self.capabilities.insert(capability);
        self
    }

    /// Sets the provenance/evidence reference.
    pub fn evidence_ref(mut self, evidence_ref: impl Into<String>) -> Self {
        self.evidence_ref = Some(evidence_ref.into());
        self
    }

    /// Builds and validates the profile.
    pub fn build(self) -> Result<CompatibilityProfile, CompatibilityError> {
        let evidence_ref = normalize_component(self.evidence_ref, "evidence_ref")?;
        let profile = CompatibilityProfile {
            id: self.id,
            revision: self.revision,
            standard_version: self.standard_version,
            match_key: self.match_key,
            digest: self.digest,
            charset: self.charset,
            endpoint: self.endpoint,
            catalog: self.catalog,
            sdp: self.sdp,
            media_status: self.media_status,
            capabilities: self.capabilities,
            evidence_ref,
        };
        profile.validate()?;
        Ok(profile)
    }
}

/// A profile snapshot pinned to a running session.
///
/// Holding a `PinnedProfile` guarantees the session keeps observing the exact
/// override semantics that were active when it started, even if the registry is
/// hot-reloaded with a newer revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedProfile {
    profile: CompatibilityProfile,
}

impl PinnedProfile {
    /// The pinned profile id.
    pub fn id(&self) -> &ProfileId {
        self.profile.id()
    }

    /// The pinned revision.
    pub fn revision(&self) -> ProfileRevision {
        self.profile.revision()
    }

    /// The pinned profile snapshot.
    pub fn profile(&self) -> &CompatibilityProfile {
        &self.profile
    }

    /// Whether a freshly loaded profile differs from this pinned snapshot, i.e.
    /// the id matches but the revision advanced. A running session should ignore
    /// the new revision until it starts a new session.
    pub fn is_superseded_by(&self, current: &CompatibilityProfile) -> bool {
        self.profile.id() == current.id() && current.revision() > self.profile.revision()
    }
}

/// The deserialized configuration form of a compatibility profile.
///
/// Mirrors the `[[gb28181.compatibility_profiles]]` TOML table. Deserialization
/// is intentionally decoupled from the validated [`CompatibilityProfile`]: call
/// [`CompatibilityProfile::from_config`] to obtain an invariant-checked profile.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityProfileConfig {
    /// Stable profile identifier.
    pub id: String,
    /// Monotonic revision (defaults to 1).
    #[serde(default = "default_revision")]
    pub revision: u32,
    /// Standard version token (`"2016"` or `"2022"`).
    pub standard_version: String,
    /// Optional manufacturer match component.
    #[serde(default)]
    pub manufacturer: Option<String>,
    /// Optional model match component (requires `manufacturer`).
    #[serde(default)]
    pub model: Option<String>,
    /// Optional firmware match component (requires `model`).
    #[serde(default)]
    pub firmware: Option<String>,
    /// Optional reference to fixture provenance metadata.
    #[serde(default)]
    pub evidence_ref: Option<String>,
    /// Declared capabilities (see [`CompatibilityCapability`]).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Digest algorithm preference token.
    #[serde(default)]
    pub digest_algorithm: Option<String>,
    /// Charset preference token.
    #[serde(default)]
    pub charset: Option<String>,
    /// `rport` handling token.
    #[serde(default)]
    pub rport: Option<String>,
    /// Response-route derivation token.
    #[serde(default)]
    pub source_route: Option<String>,
    /// Catalog fragment size (devices per fragment).
    #[serde(default)]
    pub catalog_fragment_size: Option<u16>,
    /// Whether to emit an SSRC line in generated SDP.
    #[serde(default)]
    pub sdp_emit_ssrc: Option<bool>,
    /// SDP TCP setup preference token.
    #[serde(default)]
    pub sdp_setup: Option<String>,
    /// Whether the peer sends media before ACK.
    #[serde(default)]
    pub sdp_send_media_before_ack: Option<bool>,
    /// Whether MediaStatus NotifyType=121 is accepted.
    #[serde(default)]
    pub media_status_accept_121: Option<bool>,
    /// Whether the voice broadcast handshake is enabled.
    #[serde(default)]
    pub broadcast_handshake: Option<bool>,
}

fn default_revision() -> u32 {
    ProfileRevision::INITIAL.get()
}
