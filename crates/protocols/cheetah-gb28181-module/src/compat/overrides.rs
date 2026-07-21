//! Controlled override value types carried by a [`CompatibilityProfile`].
//!
//! [`CompatibilityProfile`]: super::CompatibilityProfile

use crate::config::CharsetPolicy;
use cheetah_gb28181_core::DigestAlgorithm;

use super::CompatibilityError;

/// Preferred digest algorithm for a compatibility profile.
///
/// This mirrors the safe subset of [`DigestAlgorithm`]; it deliberately cannot
/// disable authentication, only choose between the standard algorithms and
/// whether legacy MD5 is tolerated for a specific vendor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DigestAlgorithmPreference {
    /// Prefer RFC 7616 SHA-256.
    Sha256,
    /// Prefer RFC 2617 legacy MD5 (some GB/T 28181-2016 devices only speak MD5).
    Md5,
}

impl DigestAlgorithmPreference {
    /// Parses the preference from its configuration token.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sha256" | "sha-256" => Some(Self::Sha256),
            "md5" => Some(Self::Md5),
            _ => None,
        }
    }

    /// Maps the preference to the core [`DigestAlgorithm`].
    pub fn to_core(self) -> DigestAlgorithm {
        match self {
            Self::Sha256 => DigestAlgorithm::Sha256,
            Self::Md5 => DigestAlgorithm::Md5,
        }
    }

    /// Whether this preference requires MD5 to be permitted.
    pub fn requires_md5(self) -> bool {
        matches!(self, Self::Md5)
    }
}

/// Character-set preference for GB28181 XML bodies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharsetPreference {
    /// Strict UTF-8 only.
    Utf8Strict,
    /// Allow GB2312/GBK declarations and transcode to UTF-8 before parsing.
    GbkCompatible,
}

impl CharsetPreference {
    /// Parses the preference from its configuration token.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "utf8" | "utf-8" | "utf8_strict" => Some(Self::Utf8Strict),
            "gbk" | "gbk_compatible" | "gb2312" => Some(Self::GbkCompatible),
            _ => None,
        }
    }

    /// Maps the preference to the domain [`CharsetPolicy`].
    pub fn to_policy(self) -> CharsetPolicy {
        match self {
            Self::Utf8Strict => CharsetPolicy::Utf8,
            Self::GbkCompatible => CharsetPolicy::GbkCompatible,
        }
    }
}

/// How the `rport` (RFC 3581) parameter is handled for a peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RportPolicy {
    /// Ignore `rport`; route using the dialog/transaction target only.
    Ignore,
    /// Honor `rport` when the peer offers it, otherwise fall back to the
    /// dialog target.
    Honor,
}

impl RportPolicy {
    /// Parses the policy from its configuration token.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ignore" => Some(Self::Ignore),
            "honor" => Some(Self::Honor),
            _ => None,
        }
    }
}

/// How the response route (send target) is derived for a peer.
///
/// Only authenticated REGISTER, dialog target refresh, or this explicit
/// compatibility choice may change the send route; ordinary keepalive/MESSAGE
/// never rewrites the endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRoutePolicy {
    /// Use the dialog/transaction remote target (default, safest).
    DialogTarget,
    /// Use the observed UDP source `(address, port)` of the last request. Some
    /// non-compliant devices behind NAT never advertise a reachable Contact.
    ObservedSource,
}

impl SourceRoutePolicy {
    /// Parses the policy from its configuration token.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dialog_target" | "dialog" => Some(Self::DialogTarget),
            "observed_source" | "observed" | "source" => Some(Self::ObservedSource),
            _ => None,
        }
    }
}

/// Endpoint / routing behavior overrides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointBehavior {
    rport: RportPolicy,
    source_route: SourceRoutePolicy,
}

impl EndpointBehavior {
    /// Creates an endpoint behavior override.
    pub fn new(rport: RportPolicy, source_route: SourceRoutePolicy) -> Self {
        Self {
            rport,
            source_route,
        }
    }

    /// The `rport` handling policy.
    pub fn rport(&self) -> RportPolicy {
        self.rport
    }

    /// The response-route derivation policy.
    pub fn source_route(&self) -> SourceRoutePolicy {
        self.source_route
    }
}

impl Default for EndpointBehavior {
    fn default() -> Self {
        Self {
            rport: RportPolicy::Honor,
            source_route: SourceRoutePolicy::DialogTarget,
        }
    }
}

/// Catalog behavior overrides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CatalogOverrides {
    fragment_size: u16,
}

impl CatalogOverrides {
    /// Maximum devices per catalog fragment that a profile may request. This is
    /// a hard safety ceiling; profiles cannot use it to disable fragmentation.
    pub const MAX_FRAGMENT_SIZE: u16 = 1000;

    /// Default number of catalog items per fragment.
    pub const DEFAULT_FRAGMENT_SIZE: u16 = 128;

    /// Creates catalog overrides, validating the fragment size is within
    /// `1..=MAX_FRAGMENT_SIZE`.
    pub fn new(fragment_size: u16) -> Result<Self, CompatibilityError> {
        if fragment_size == 0 || fragment_size > Self::MAX_FRAGMENT_SIZE {
            return Err(CompatibilityError::InvalidCatalogFragmentSize {
                max: Self::MAX_FRAGMENT_SIZE,
            });
        }
        Ok(Self { fragment_size })
    }

    /// Number of catalog items emitted per fragment.
    pub fn fragment_size(&self) -> u16 {
        self.fragment_size
    }
}

impl Default for CatalogOverrides {
    fn default() -> Self {
        Self {
            fragment_size: Self::DEFAULT_FRAGMENT_SIZE,
        }
    }
}

/// SDP media transport setup preference for controlled overrides.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdpSetupPreference {
    /// Do not force a setup role; use the negotiated default.
    Negotiated,
    /// Prefer active TCP setup.
    ActiveTcp,
    /// Prefer passive TCP setup.
    PassiveTcp,
}

impl SdpSetupPreference {
    /// Parses the preference from its configuration token.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "negotiated" | "default" => Some(Self::Negotiated),
            "active" | "active_tcp" => Some(Self::ActiveTcp),
            "passive" | "passive_tcp" => Some(Self::PassiveTcp),
            _ => None,
        }
    }
}

/// SDP overrides for a compatibility profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SdpOverrides {
    emit_ssrc: bool,
    setup: SdpSetupPreference,
    send_media_before_ack: bool,
}

impl SdpOverrides {
    /// Creates SDP overrides.
    pub fn new(emit_ssrc: bool, setup: SdpSetupPreference, send_media_before_ack: bool) -> Self {
        Self {
            emit_ssrc,
            setup,
            send_media_before_ack,
        }
    }

    /// Whether an `y=` SSRC line is emitted in generated SDP.
    pub fn emit_ssrc(&self) -> bool {
        self.emit_ssrc
    }

    /// The TCP setup role preference.
    pub fn setup(&self) -> SdpSetupPreference {
        self.setup
    }

    /// Whether the peer sends media before the ACK completes (some devices
    /// start streaming immediately after the 200 OK).
    pub fn send_media_before_ack(&self) -> bool {
        self.send_media_before_ack
    }
}

impl Default for SdpOverrides {
    fn default() -> Self {
        Self {
            emit_ssrc: true,
            setup: SdpSetupPreference::Negotiated,
            send_media_before_ack: false,
        }
    }
}

/// MediaStatus / broadcast behavior overrides.
///
/// Both overrides default to `false`, i.e. the strict standard behavior.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MediaStatusOverrides {
    accept_status_code_121: bool,
    broadcast_handshake: bool,
}

impl MediaStatusOverrides {
    /// Creates MediaStatus overrides.
    pub fn new(accept_status_code_121: bool, broadcast_handshake: bool) -> Self {
        Self {
            accept_status_code_121,
            broadcast_handshake,
        }
    }

    /// Whether a MediaStatus `NotifyType=121` (end of file) is accepted as a
    /// terminal media event for this peer.
    pub fn accept_status_code_121(&self) -> bool {
        self.accept_status_code_121
    }

    /// Whether the multi-step voice broadcast handshake is enabled.
    pub fn broadcast_handshake(&self) -> bool {
        self.broadcast_handshake
    }
}
