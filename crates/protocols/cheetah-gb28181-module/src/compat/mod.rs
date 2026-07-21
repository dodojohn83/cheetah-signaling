//! Explicit, auditable GB28181 compatibility profiles.
//!
//! A [`CompatibilityProfile`] captures the controlled, per-vendor deviations
//! that a deployment is willing to accept from a specific device or platform
//! (standard version, digest algorithm, charset preference, endpoint routing,
//! catalog fragmentation and SDP/MediaStatus behavior). Profiles are pure data
//! plus validation and selection logic; this module performs no network,
//! database or clock I/O and therefore stays protocol-core clean.
//!
//! # Design
//!
//! - Profiles only expose *controlled* override knobs. The schema deliberately
//!   provides no way to relax tenant/identity/Digest/owner-epoch checks, CRLF or
//!   XML DTD/entity limits, body/depth/queue bounds, or the success semantics of
//!   a shared public operation. Those invariants are enforced by construction:
//!   there simply is no field to weaken them.
//! - Selection uses a fixed priority: exact firmware → model → manufacturer →
//!   standard generic. Two profiles that would match the same device at the same
//!   priority are a configuration error, surfaced as
//!   [`CompatibilityError::AmbiguousMatch`].
//! - A resolved profile is pinned to a [`ProfileRevision`] via
//!   [`CompatibilityProfile::pin`]. A running dialog holds a [`PinnedProfile`]
//!   snapshot so a hot configuration reload never changes the semantics of an
//!   in-flight session.
//!
//! Wiring the individual overrides into the charset/MIME/header/endpoint,
//! catalog and SDP/MediaStatus code paths is handled by later tasks
//! (`GB4-COMP-002`, `GB4-COMP-003`); this module defines the schema, validation,
//! capability negotiation and revision pinning (`GB4-COMP-001`).

mod overrides;
mod profile;
mod registry;

#[cfg(test)]
mod tests;

pub use overrides::{
    CatalogOverrides, CharsetPreference, DigestAlgorithmPreference, EndpointBehavior,
    MediaStatusOverrides, RportPolicy, SdpOverrides, SdpSetupPreference, SourceRoutePolicy,
};
pub use profile::{
    CompatibilityProfile, CompatibilityProfileConfig, PinnedProfile, ProfileId, ProfileMatchKey,
    ProfileRevision,
};
pub use registry::{CompatibilityRegistry, DeviceDescriptor, MatchSpecificity, ProfileSelection};

/// The GB28181 standard revision a device or platform speaks.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StandardVersion {
    /// GB/T 28181-2016.
    Gb2016,
    /// GB/T 28181-2022.
    Gb2022,
}

impl StandardVersion {
    /// Parses a standard version from its configuration string.
    ///
    /// Accepts `"2016"`/`"gb2016"` and `"2022"`/`"gb2022"` case-insensitively.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "2016" | "gb2016" | "gb/t 28181-2016" | "gb28181-2016" => Some(Self::Gb2016),
            "2022" | "gb2022" | "gb/t 28181-2022" | "gb28181-2022" => Some(Self::Gb2022),
            _ => None,
        }
    }

    /// Returns the canonical short label (`"2016"` or `"2022"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gb2016 => "2016",
            Self::Gb2022 => "2022",
        }
    }
}

impl std::fmt::Display for StandardVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A controlled compatibility capability a profile may advertise.
///
/// Capabilities are opt-in behaviors that must be explicitly enabled by a
/// profile before the runtime negotiates them for a device/platform. They never
/// relax safety invariants; they only gate optional, standard-defined features.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum CompatibilityCapability {
    /// IPv6 transport is supported for this peer.
    Ipv6Transport,
    /// `ConfigDownload` device configuration query is supported.
    ConfigDownload,
    /// `PresetQuery` PTZ preset enumeration is supported.
    PresetQuery,
    /// Voice broadcast (`Broadcast`) handshake is supported.
    Broadcast,
    /// `MediaStatus` end-of-stream notifications are honored.
    MediaStatusReport,
    /// Catalog change `Notify` is supported.
    CatalogNotify,
    /// Alarm event subscription is supported.
    AlarmSubscription,
    /// Mobile-position event subscription is supported.
    MobilePositionSubscription,
    /// Registering to multiple upstream platforms is supported.
    MultipleUpstream,
    /// Virtual organization / directory synthesis is supported.
    VirtualDirectory,
    /// Pre-configured custom external IDs are supported.
    CustomExternalId,
}

impl CompatibilityCapability {
    /// Parses a capability from its configuration token (case-insensitive,
    /// dashes and underscores are equivalent).
    pub fn parse(value: &str) -> Option<Self> {
        let normalized: String = value
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c == '-' { '_' } else { c })
            .collect();
        match normalized.as_str() {
            "ipv6" | "ipv6_transport" => Some(Self::Ipv6Transport),
            "config_download" => Some(Self::ConfigDownload),
            "preset_query" => Some(Self::PresetQuery),
            "broadcast" => Some(Self::Broadcast),
            "media_status" | "media_status_report" => Some(Self::MediaStatusReport),
            "catalog_notify" => Some(Self::CatalogNotify),
            "alarm_subscription" => Some(Self::AlarmSubscription),
            "mobile_position_subscription" => Some(Self::MobilePositionSubscription),
            "multiple_upstream" => Some(Self::MultipleUpstream),
            "virtual_directory" => Some(Self::VirtualDirectory),
            "custom_external_id" => Some(Self::CustomExternalId),
            _ => None,
        }
    }

    /// Returns the canonical configuration token for this capability.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ipv6Transport => "ipv6_transport",
            Self::ConfigDownload => "config_download",
            Self::PresetQuery => "preset_query",
            Self::Broadcast => "broadcast",
            Self::MediaStatusReport => "media_status_report",
            Self::CatalogNotify => "catalog_notify",
            Self::AlarmSubscription => "alarm_subscription",
            Self::MobilePositionSubscription => "mobile_position_subscription",
            Self::MultipleUpstream => "multiple_upstream",
            Self::VirtualDirectory => "virtual_directory",
            Self::CustomExternalId => "custom_external_id",
        }
    }
}

impl std::fmt::Display for CompatibilityCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Errors produced while validating, indexing or selecting compatibility
/// profiles.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CompatibilityError {
    /// The profile identifier is empty, too long, or contains illegal
    /// characters.
    #[error("invalid profile id")]
    InvalidProfileId,
    /// A match-key component is empty, too long, or violates the
    /// manufacturer → model → firmware hierarchy.
    #[error("invalid match key: {0}")]
    InvalidMatchKey(&'static str),
    /// The catalog fragment size is zero or exceeds the hard maximum.
    #[error("catalog fragment size must be in 1..={max}")]
    InvalidCatalogFragmentSize {
        /// The maximum allowed catalog fragment size.
        max: u16,
    },
    /// An unknown capability token was supplied in configuration.
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    /// An unknown standard-version token was supplied in configuration.
    #[error("unknown standard version: {0}")]
    UnknownStandardVersion(String),
    /// An unknown enumerated override token was supplied in configuration.
    #[error("unknown {field} value: {value}")]
    UnknownOverrideValue {
        /// The configuration field that failed to parse.
        field: &'static str,
        /// The offending value.
        value: String,
    },
    /// Two profiles share the same id.
    #[error("duplicate profile id: {0}")]
    DuplicateProfileId(String),
    /// Two profiles declare the same standard version and match key, so a
    /// device could match both at the same priority.
    #[error("duplicate match key for standard {standard}")]
    DuplicateMatchKey {
        /// The standard version shared by the conflicting profiles.
        standard: StandardVersion,
    },
    /// Multiple profiles matched a device at the same priority.
    #[error("ambiguous profile match at {specificity:?} priority")]
    AmbiguousMatch {
        /// The priority level at which the ambiguity occurred.
        specificity: MatchSpecificity,
    },
}
