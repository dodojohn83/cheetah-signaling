//! Controlled media-negotiation overrides carried by a compatibility profile.
//!
//! `GB4-COMP-003` extends the [`CompatibilityProfile`](crate::CompatibilityProfile)
//! schema with three additional, explicitly-scoped override categories:
//!
//! * SDP media negotiation — extra RTP payload types and vendor `a=` attribute
//!   names a non-standard device is allowed to answer with;
//! * broadcast (voice) address handling — where the broadcast/talk media
//!   connection address is anchored;
//! * `MediaStatus` response normalisation — vendor `NotifyType` values that map
//!   to the canonical end-of-stream outcome.
//!
//! These overrides never carry, parse or store RTP/RTCP/PS/TS/ES payloads: they
//! only describe how the signaling plane negotiates addresses and payload
//! *identifiers* through the typed `cheetah.media.v1` MediaPort contracts. All
//! evaluation is pure and side-effect free; the gating on device/firmware match
//! and capability is applied by [`CompatibilityProfile`](crate::CompatibilityProfile).

use crate::DomainError;

/// Maximum number of entries in any single override list.
pub const MAX_OVERRIDE_ENTRIES: usize = 64;

/// Maximum byte length of an individual override list entry.
pub const MAX_OVERRIDE_ENTRY_BYTES: usize = 64;

/// The set of controlled media overrides declared by a compatibility profile.
///
/// Each override is optional; a `None` value means "no override, use the strict
/// default behaviour". The corresponding capability must still be enabled for
/// the override to take effect (see the accessor methods on
/// [`CompatibilityProfile`](crate::CompatibilityProfile)).
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CompatibilityOverrides {
    /// SDP media negotiation override.
    pub sdp: Option<SdpMediaOverride>,
    /// Broadcast/talk address handling override.
    pub broadcast: Option<BroadcastOverride>,
    /// `MediaStatus` response normalisation override.
    pub media_status: Option<MediaStatusOverride>,
}

impl CompatibilityOverrides {
    /// Validates the declared overrides against the configured bounds.
    pub fn validate(&self) -> crate::Result<()> {
        if let Some(sdp) = &self.sdp {
            sdp.validate()?;
        }
        if let Some(media_status) = &self.media_status {
            media_status.validate()?;
        }
        Ok(())
    }
}

/// Extra SDP media elements a non-standard device is permitted to answer with.
///
/// The strict default accepts only the GB28181 baseline payload types and the
/// typed attributes the SDP parser recognises. This override widens that set for
/// a specific device/firmware profile without loosening the generic parser.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SdpMediaOverride {
    /// Additional RTP payload-type numbers (decimal strings) tolerated in a
    /// device SDP answer beyond the GB28181 baseline.
    pub allowed_payload_types: Vec<String>,
    /// Additional vendor `a=` attribute names (case-insensitive) tolerated in a
    /// device SDP answer beyond the recognised/baseline set.
    pub allowed_attribute_names: Vec<String>,
}

impl SdpMediaOverride {
    fn validate(&self) -> crate::Result<()> {
        validate_entries("sdp.allowed_payload_types", &self.allowed_payload_types)?;
        validate_entries("sdp.allowed_attribute_names", &self.allowed_attribute_names)?;
        Ok(())
    }

    /// Returns `true` if the payload type is explicitly allowed by this override.
    pub fn allows_payload(&self, payload_type: &str) -> bool {
        self.allowed_payload_types.iter().any(|p| p == payload_type)
    }

    /// Returns `true` if the vendor attribute name is explicitly allowed.
    pub fn allows_attribute(&self, name: &str) -> bool {
        self.allowed_attribute_names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Where the broadcast/talk media connection address is anchored.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BroadcastAddressSource {
    /// Use the media node address negotiated through the MediaPort (default).
    #[default]
    MediaNode,
    /// Anchor the broadcast media connection at the signaling host advertised to
    /// the device. Required by some intercom devices that dial audio back to the
    /// SIP server rather than to the media node.
    SignalingHost,
}

impl std::str::FromStr for BroadcastAddressSource {
    type Err = DomainError;

    fn from_str(s: &str) -> crate::Result<Self> {
        if crate::from_str_helpers::eq_normalized_snake(s, "media_node") {
            Ok(Self::MediaNode)
        } else if crate::from_str_helpers::eq_normalized_snake(s, "signaling_host") {
            Ok(Self::SignalingHost)
        } else {
            let display = crate::from_str_helpers::truncate_for_error(s);
            Err(DomainError::invalid_argument(format!(
                "unknown broadcast address source: {display}"
            )))
        }
    }
}

/// Broadcast/talk address handling override.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BroadcastOverride {
    /// Address source used when building the broadcast/talk SDP offer.
    pub address_source: BroadcastAddressSource,
}

/// `MediaStatus` response normalisation override.
///
/// The GB28181 canonical end-of-stream `NotifyType` is `121`. Some vendors emit
/// a different value; this override lists the extra `NotifyType` values that must
/// be normalised to the canonical stopped outcome for the matched profile.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MediaStatusOverride {
    /// Vendor `NotifyType` values that mean "stream stopped/finished" in
    /// addition to the canonical `121`.
    pub stopped_status_codes: Vec<String>,
}

impl MediaStatusOverride {
    fn validate(&self) -> crate::Result<()> {
        validate_entries(
            "media_status.stopped_status_codes",
            &self.stopped_status_codes,
        )
    }

    /// Returns `true` if the `NotifyType` value is a vendor stopped code.
    pub fn is_stopped(&self, notify_type: &str) -> bool {
        self.stopped_status_codes.iter().any(|c| c == notify_type)
    }
}

/// Normalised outcome of a GB28181 `MediaStatus` notification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaStatusOutcome {
    /// The media stream has stopped/finished (canonical `NotifyType` 121 or a
    /// configured vendor equivalent).
    Stopped,
    /// The `NotifyType` value is not a recognised stopped code.
    Unknown,
}

/// The canonical GB28181 `MediaStatus` end-of-stream `NotifyType`.
pub const MEDIA_STATUS_STOPPED_NOTIFY_TYPE: &str = "121";

fn validate_entries(field: &str, entries: &[String]) -> crate::Result<()> {
    if entries.len() > MAX_OVERRIDE_ENTRIES {
        return Err(DomainError::invalid_argument(format!(
            "{field} must not exceed {MAX_OVERRIDE_ENTRIES} entries"
        )));
    }
    for entry in entries {
        if entry.is_empty() {
            return Err(DomainError::invalid_argument(format!(
                "{field} entries must not be empty"
            )));
        }
        if entry.len() > MAX_OVERRIDE_ENTRY_BYTES {
            return Err(DomainError::invalid_argument(format!(
                "{field} entry too long"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{CompatibilityCapability, CompatibilityProfile};

    fn profile(
        capabilities: Vec<CompatibilityCapability>,
        overrides: CompatibilityOverrides,
    ) -> CompatibilityProfile {
        CompatibilityProfile {
            capabilities,
            overrides,
            ..Default::default()
        }
    }

    #[test]
    fn baseline_sdp_payloads_are_always_allowed() {
        let default = CompatibilityProfile::default();
        for pt in ["0", "8", "96", "97", "126", "127"] {
            assert!(default.sdp_payload_allowed(pt), "{pt} should be baseline");
        }
    }

    #[test]
    fn non_baseline_payload_rejected_without_override() {
        let default = CompatibilityProfile::default();
        assert!(!default.sdp_payload_allowed("34"));
        assert!(!default.sdp_payload_allowed("200"));
        assert!(!default.sdp_payload_allowed("not-a-number"));
    }

    #[test]
    fn payload_override_requires_capability() {
        let overrides = CompatibilityOverrides {
            sdp: Some(SdpMediaOverride {
                allowed_payload_types: vec!["34".to_string()],
                allowed_attribute_names: Vec::new(),
            }),
            ..Default::default()
        };
        // Override present but capability missing: still rejected.
        let without_cap = profile(Vec::new(), overrides.clone());
        assert!(!without_cap.sdp_payload_allowed("34"));

        let with_cap = profile(vec![CompatibilityCapability::SdpMediaOverride], overrides);
        assert!(with_cap.sdp_payload_allowed("34"));
        assert!(!with_cap.sdp_payload_allowed("35"));
    }

    #[test]
    fn attribute_override_requires_capability_and_listing() {
        let overrides = CompatibilityOverrides {
            sdp: Some(SdpMediaOverride {
                allowed_payload_types: Vec::new(),
                allowed_attribute_names: vec!["vendorext".to_string()],
            }),
            ..Default::default()
        };
        let default = CompatibilityProfile::default();
        assert!(default.sdp_attribute_allowed("downloadspeed"));
        assert!(!default.sdp_attribute_allowed("vendorext"));

        let with_cap = profile(vec![CompatibilityCapability::SdpMediaOverride], overrides);
        assert!(with_cap.sdp_attribute_allowed("VendorExt"));
        assert!(!with_cap.sdp_attribute_allowed("other"));
    }

    #[test]
    fn broadcast_address_source_gated_by_capability() {
        let overrides = CompatibilityOverrides {
            broadcast: Some(BroadcastOverride {
                address_source: BroadcastAddressSource::SignalingHost,
            }),
            ..Default::default()
        };
        let without_cap = profile(Vec::new(), overrides.clone());
        assert_eq!(
            without_cap.broadcast_address_source(),
            BroadcastAddressSource::MediaNode
        );

        let with_cap = profile(vec![CompatibilityCapability::Broadcast], overrides);
        assert_eq!(
            with_cap.broadcast_address_source(),
            BroadcastAddressSource::SignalingHost
        );
    }

    #[test]
    fn media_status_outcome_gated_and_normalised() {
        let overrides = CompatibilityOverrides {
            media_status: Some(MediaStatusOverride {
                stopped_status_codes: vec!["99".to_string()],
            }),
            ..Default::default()
        };
        // No capability: even canonical 121 is Unknown (feature off).
        let without_cap = profile(Vec::new(), overrides.clone());
        assert_eq!(
            without_cap.media_status_outcome("121"),
            MediaStatusOutcome::Unknown
        );

        let with_cap = profile(vec![CompatibilityCapability::MediaStatus], overrides);
        assert_eq!(
            with_cap.media_status_outcome("121"),
            MediaStatusOutcome::Stopped
        );
        assert_eq!(
            with_cap.media_status_outcome(" 99 "),
            MediaStatusOutcome::Stopped
        );
        assert_eq!(
            with_cap.media_status_outcome("7"),
            MediaStatusOutcome::Unknown
        );
    }

    #[test]
    fn validate_rejects_oversized_and_empty_entries() {
        let too_many = SdpMediaOverride {
            allowed_payload_types: (0..(MAX_OVERRIDE_ENTRIES + 1))
                .map(|i| i.to_string())
                .collect(),
            allowed_attribute_names: Vec::new(),
        };
        assert!(too_many.validate().is_err());

        let empty = MediaStatusOverride {
            stopped_status_codes: vec![String::new()],
        };
        assert!(empty.validate().is_err());

        let long = MediaStatusOverride {
            stopped_status_codes: vec!["x".repeat(MAX_OVERRIDE_ENTRY_BYTES + 1)],
        };
        assert!(long.validate().is_err());
    }

    #[test]
    fn broadcast_address_source_parses() {
        assert_eq!(
            "signaling-host".parse::<BroadcastAddressSource>().unwrap(),
            BroadcastAddressSource::SignalingHost
        );
        assert_eq!(
            "media_node".parse::<BroadcastAddressSource>().unwrap(),
            BroadcastAddressSource::MediaNode
        );
        assert!("bogus".parse::<BroadcastAddressSource>().is_err());
    }
}
