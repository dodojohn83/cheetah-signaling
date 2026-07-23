//! Persistent protocol session aggregate.
//!
//! A [`ProtocolSession`] models a GB28181 REGISTER binding or a short-lived
//! control session. It is deliberately kept separate from `Operation` and
//! `MediaSession`: it captures the *transport and registration* facts a
//! signaling node needs to route SIP to a device (Contact, endpoint, Call-ID,
//! CSeq, expiry) together with presence and ownership fencing state.
//!
//! The aggregate is transport-typed but does not itself perform any I/O; the
//! REGISTER / keepalive transaction chain that drives its transitions lives in
//! the application layer (see `GB4-ACC-002`).

use crate::compatibility::{
    BroadcastAddressSource, CompatibilityOverrides, MEDIA_STATUS_STOPPED_NOTIFY_TYPE,
    MediaStatusOutcome,
};
use crate::{DomainError, Protocol};
use cheetah_signal_types::{
    Clock, DeviceId, NodeId, OwnerEpoch, ProtocolIdentity, ProtocolSessionId, Revision, TenantId,
    UtcTimestamp,
};

/// Maximum byte length of the free-form string fields carried by a session.
const MAX_FIELD_BYTES: usize = 512;

/// SIP transport used by a protocol session.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SipTransport {
    /// UDP transport.
    #[default]
    Udp,
    /// TCP transport.
    Tcp,
}

impl std::fmt::Display for SipTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for SipTransport {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let transport = if s.eq_ignore_ascii_case("udp") {
            Self::Udp
        } else if s.eq_ignore_ascii_case("tcp") {
            Self::Tcp
        } else {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown transport: {display}"
            )));
        };
        Ok(transport)
    }
}

/// Presence of the device behind a protocol session.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PresenceState {
    /// Presence has not yet been determined.
    #[default]
    Unknown,
    /// The device is online (registered and keepalive current).
    Online,
    /// The device is offline.
    Offline,
}

impl std::fmt::Display for PresenceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Unknown => "unknown",
            Self::Online => "online",
            Self::Offline => "offline",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for PresenceState {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let state = if s.eq_ignore_ascii_case("unknown") {
            Self::Unknown
        } else if s.eq_ignore_ascii_case("online") {
            Self::Online
        } else if s.eq_ignore_ascii_case("offline") {
            Self::Offline
        } else {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown presence state: {display}"
            )));
        };
        Ok(state)
    }
}

/// Local listener identity that terminated the registration.
///
/// These values come from listener/domain routing and must uniquely resolve to
/// a tenant before a session is created.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalIdentity {
    /// Configured listener id that accepted the registration.
    pub listener_id: String,
    /// Local platform (SIP server) device id, e.g. `34020000002000000001`.
    pub local_device_id: String,
    /// SIP domain.
    pub domain: String,
    /// Digest realm.
    pub realm: String,
}

impl LocalIdentity {
    fn validate(&self) -> crate::Result<()> {
        for (name, value) in [
            ("listener_id", &self.listener_id),
            ("local_device_id", &self.local_device_id),
            ("domain", &self.domain),
            ("realm", &self.realm),
        ] {
            if value.is_empty() {
                return Err(DomainError::invalid_argument(format!(
                    "{name} must not be empty"
                )));
            }
            if value.len() > MAX_FIELD_BYTES {
                return Err(DomainError::invalid_argument(format!("{name} too long")));
            }
        }
        Ok(())
    }
}

/// Endpoint routing facts for a protocol session.
///
/// The distinct fields intentionally separate what was *observed* on the wire
/// from what the device *advertised*, so NAT/rport handling and source-hijack
/// checks stay explicit rather than collapsing into a single "current source".
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SessionEndpoint {
    /// Source address:port observed on the received packet.
    pub observed_source: String,
    /// Contact URI advertised by the device.
    pub contact_uri: String,
    /// Endpoint the node advertises back to the device.
    pub advertised_endpoint: String,
}

impl SessionEndpoint {
    fn validate(&self) -> crate::Result<()> {
        for (name, value) in [
            ("observed_source", &self.observed_source),
            ("contact_uri", &self.contact_uri),
            ("advertised_endpoint", &self.advertised_endpoint),
        ] {
            if value.len() > MAX_FIELD_BYTES {
                return Err(DomainError::invalid_argument(format!("{name} too long")));
            }
        }
        Ok(())
    }
}

/// The SIP registration transaction facts for a protocol session.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RegistrationInfo {
    /// REGISTER Call-ID.
    pub call_id: String,
    /// Highest REGISTER CSeq seen.
    pub cseq: u32,
    /// Requested `Expires` value in seconds.
    pub expires_secs: u32,
}

impl RegistrationInfo {
    fn validate(&self) -> crate::Result<()> {
        if self.call_id.len() > MAX_FIELD_BYTES {
            return Err(DomainError::invalid_argument("call_id too long"));
        }
        Ok(())
    }
}

/// A controlled capability that may be enabled by a [`CompatibilityProfile`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CompatibilityCapability {
    /// Allow charset fallback and XML declaration mismatch.
    CharsetFallback,
    /// Recognise vendor-specific MIME aliases as typed messages.
    MimeAlias,
    /// Use Via `received`/`rport` to override the Contact route.
    ContactRportRoute,
    /// Normalise non-ambiguous malformed SIP headers.
    HeaderNormalization,
    /// Accept catalog fragments using `SumNum` rather than strict counts.
    CatalogCountFragment,
    /// Emit catalog change NOTIFY without an established subscription.
    CatalogNotify,
    /// Support Alarm event package subscription/notify.
    AlarmSubscription,
    /// Support MobilePosition event package subscription/notify.
    MobilePosition,
    /// Enable GB/T 28181-2016 extensions (IPv6, ConfigDownload, etc.).
    Gb2016,
    /// Support device configuration download queries.
    ConfigDownload,
    /// Support PTZ preset query commands.
    PresetQuery,
    /// Support broadcast commands and handshakes.
    Broadcast,
    /// Support media status reporting and parsing.
    MediaStatus,
    /// Enable SDP media negotiation overrides (extra payload types/attributes).
    SdpMediaOverride,
    /// Allow duplicate REGISTER transactions for the same device identity.
    DuplicateRegisterAllowed,
    /// Enforce strict realm/domain alignment for digest challenges.
    StrictRealm,
    /// Use a non-zero minimum REGISTER expiry in responses.
    MinimumExpiry,
    /// Prefer UDP for outgoing SIP requests.
    UdpRoute,
    /// Prefer TCP for outgoing SIP requests.
    TcpRoute,
    /// Use per-device passwords rather than a shared node secret.
    DevicePerPassword,
}

impl std::str::FromStr for CompatibilityCapability {
    type Err = DomainError;

    fn from_str(s: &str) -> crate::Result<Self> {
        let cap =
            if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "charset_fallback") {
                Self::CharsetFallback
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "mime_alias") {
                Self::MimeAlias
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "contact_rport_route",
            ) {
                Self::ContactRportRoute
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "header_normalization",
            ) {
                Self::HeaderNormalization
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "catalog_count_fragment",
            ) {
                Self::CatalogCountFragment
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "catalog_notify")
            {
                Self::CatalogNotify
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "alarm_subscription",
            ) {
                Self::AlarmSubscription
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "mobile_position",
            ) {
                Self::MobilePosition
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "gb2016") {
                Self::Gb2016
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "config_download",
            ) {
                Self::ConfigDownload
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "preset_query") {
                Self::PresetQuery
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "broadcast") {
                Self::Broadcast
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "media_status") {
                Self::MediaStatus
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "sdp_media_override",
            ) {
                Self::SdpMediaOverride
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "duplicate_register_allowed",
            ) {
                Self::DuplicateRegisterAllowed
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "strict_realm") {
                Self::StrictRealm
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "minimum_expiry")
            {
                Self::MinimumExpiry
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "udp_route") {
                Self::UdpRoute
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(s, "tcp_route") {
                Self::TcpRoute
            } else if crate::str_util::eq_ignore_ascii_case_and_dash_underscore(
                s,
                "device_per_password",
            ) {
                Self::DevicePerPassword
            } else {
                let display = s.chars().take(64).collect::<String>();
                return Err(DomainError::invalid_argument(format!(
                    "unknown compatibility capability: {display}"
                )));
            };
        Ok(cap)
    }
}

/// Compatibility profile applied to a device's protocol session.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CompatibilityProfile {
    /// Optional profile id; `None` means the default behavior.
    pub profile_id: Option<String>,
    /// GB/T 28181 standard version, e.g. `2011` or `2016`.
    pub standard_version: Option<String>,
    /// Device manufacturer name.
    pub manufacturer: Option<String>,
    /// Device model name.
    pub model: Option<String>,
    /// Device firmware version.
    pub firmware: Option<String>,
    /// Controlled capabilities enabled by this profile.
    pub capabilities: Vec<CompatibilityCapability>,
    /// Path or URL to the provenance fixture that justifies this profile.
    pub evidence_ref: Option<String>,
    /// Profile revision, used to detect profile changes and pin sessions.
    pub revision: u32,
    /// Controlled media-negotiation overrides (SDP/broadcast/MediaStatus).
    ///
    /// Each override is only applied when both this profile matches the device
    /// and the gating capability is enabled; see the accessor methods below.
    pub overrides: CompatibilityOverrides,
}

/// Profile selection criteria used when resolving the best matching profile.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileSelector {
    /// Standard version advertised by the device.
    pub standard_version: Option<String>,
    /// Manufacturer advertised by the device.
    pub manufacturer: Option<String>,
    /// Model advertised by the device.
    pub model: Option<String>,
    /// Firmware version advertised by the device.
    pub firmware: Option<String>,
}

impl CompatibilityProfile {
    fn validate(&self) -> crate::Result<()> {
        for (name, value) in [
            ("profile_id", self.profile_id.as_ref()),
            ("standard_version", self.standard_version.as_ref()),
            ("manufacturer", self.manufacturer.as_ref()),
            ("model", self.model.as_ref()),
            ("firmware", self.firmware.as_ref()),
            ("evidence_ref", self.evidence_ref.as_ref()),
        ] {
            if let Some(v) = value
                && v.len() > MAX_FIELD_BYTES
            {
                return Err(DomainError::invalid_argument(format!("{name} too long")));
            }
        }
        if self.capabilities.len() > 64 {
            return Err(DomainError::invalid_argument(
                "compatibility capabilities must not exceed 64",
            ));
        }
        self.overrides.validate()?;
        Ok(())
    }

    /// Returns the matching score against `selector`.
    ///
    /// A profile only scores if every field it sets matches the selector. Fields
    /// the profile leaves blank are ignored, so a broad `standard_version` profile
    /// still applies to a device that also reports manufacturer, model or firmware.
    ///
    /// The score is the weighted sum of matched set fields, with weights chosen so
    /// the priority order is `firmware > model > manufacturer > standard_version >
    /// default`: a profile matching a more specific field always outranks a profile
    /// that only matches less specific fields.
    pub fn score(&self, selector: &ProfileSelector) -> u32 {
        const FIRMWARE_WEIGHT: u32 = 8;
        const MODEL_WEIGHT: u32 = 4;
        const MANUFACTURER_WEIGHT: u32 = 2;
        const STANDARD_WEIGHT: u32 = 1;

        let mut score = 0u32;
        if !add_field_score(
            &mut score,
            self.firmware.as_deref(),
            selector.firmware.as_deref(),
            FIRMWARE_WEIGHT,
        ) {
            return 0;
        }
        if !add_field_score(
            &mut score,
            self.model.as_deref(),
            selector.model.as_deref(),
            MODEL_WEIGHT,
        ) {
            return 0;
        }
        if !add_field_score(
            &mut score,
            self.manufacturer.as_deref(),
            selector.manufacturer.as_deref(),
            MANUFACTURER_WEIGHT,
        ) {
            return 0;
        }
        if !add_field_score(
            &mut score,
            self.standard_version.as_deref(),
            selector.standard_version.as_deref(),
            STANDARD_WEIGHT,
        ) {
            return 0;
        }
        score
    }

    /// Returns `true` if `capability` is enabled by this profile.
    pub fn has(&self, capability: CompatibilityCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    /// Returns `true` if `payload_type` may appear in a device SDP answer.
    ///
    /// The GB28181 baseline (static `0`/`8` and the dynamic `96..=127` range) is
    /// always accepted. Any other payload type is rejected unless the
    /// [`SdpMediaOverride`](CompatibilityCapability::SdpMediaOverride) capability
    /// is enabled and lists it in [`SdpMediaOverride`](crate::SdpMediaOverride).
    pub fn sdp_payload_allowed(&self, payload_type: &str) -> bool {
        if is_baseline_payload_type(payload_type) {
            return true;
        }
        if !self.has(CompatibilityCapability::SdpMediaOverride) {
            return false;
        }
        self.overrides
            .sdp
            .as_ref()
            .is_some_and(|o| o.allows_payload(payload_type))
    }

    /// Returns `true` if a vendor `a=` attribute `name` may appear in a device
    /// SDP answer.
    ///
    /// Baseline vendor attribute names are always accepted. Any other name is
    /// rejected unless the
    /// [`SdpMediaOverride`](CompatibilityCapability::SdpMediaOverride) capability
    /// is enabled and lists it.
    pub fn sdp_attribute_allowed(&self, name: &str) -> bool {
        if is_baseline_attribute(name) {
            return true;
        }
        if !self.has(CompatibilityCapability::SdpMediaOverride) {
            return false;
        }
        self.overrides
            .sdp
            .as_ref()
            .is_some_and(|o| o.allows_attribute(name))
    }

    /// Returns the address source used to build a broadcast/talk SDP offer.
    ///
    /// Defaults to [`BroadcastAddressSource::MediaNode`]; the override is only
    /// honoured when the [`Broadcast`](CompatibilityCapability::Broadcast)
    /// capability is enabled.
    pub fn broadcast_address_source(&self) -> BroadcastAddressSource {
        if !self.has(CompatibilityCapability::Broadcast) {
            return BroadcastAddressSource::MediaNode;
        }
        self.overrides
            .broadcast
            .as_ref()
            .map(|o| o.address_source)
            .unwrap_or_default()
    }

    /// Normalises a GB28181 `MediaStatus` `NotifyType` value into an outcome.
    ///
    /// Returns [`MediaStatusOutcome::Unknown`] unless the
    /// [`MediaStatus`](CompatibilityCapability::MediaStatus) capability is
    /// enabled. With the capability the canonical `121` value and any configured
    /// vendor code normalise to [`MediaStatusOutcome::Stopped`].
    pub fn media_status_outcome(&self, notify_type: &str) -> MediaStatusOutcome {
        if !self.has(CompatibilityCapability::MediaStatus) {
            return MediaStatusOutcome::Unknown;
        }
        let notify_type = notify_type.trim();
        if notify_type == MEDIA_STATUS_STOPPED_NOTIFY_TYPE {
            return MediaStatusOutcome::Stopped;
        }
        let vendor_stopped = self
            .overrides
            .media_status
            .as_ref()
            .is_some_and(|o| o.is_stopped(notify_type));
        if vendor_stopped {
            MediaStatusOutcome::Stopped
        } else {
            MediaStatusOutcome::Unknown
        }
    }
}

/// Returns `true` for RTP payload types in the GB28181 baseline set: the static
/// `0` (PCMU) and `8` (PCMA) plus the dynamic `96..=127` range.
fn is_baseline_payload_type(payload_type: &str) -> bool {
    match payload_type.parse::<u16>() {
        Ok(0) | Ok(8) => true,
        Ok(pt) => (96..=127).contains(&pt),
        Err(_) => false,
    }
}

/// Baseline vendor `a=` attribute names accepted without an override.
const BASELINE_SDP_ATTRIBUTES: &[&str] = &["downloadspeed", "filesize", "username", "password"];

/// Returns `true` for vendor attribute names accepted without an override.
fn is_baseline_attribute(name: &str) -> bool {
    BASELINE_SDP_ATTRIBUTES
        .iter()
        .any(|b| b.eq_ignore_ascii_case(name))
}

fn add_field_score(
    score: &mut u32,
    profile_value: Option<&str>,
    selector_value: Option<&str>,
    weight: u32,
) -> bool {
    match (profile_value, selector_value) {
        (Some(p), Some(s)) => {
            if p.eq_ignore_ascii_case(s.trim()) {
                *score += weight;
                true
            } else {
                false
            }
        }
        (Some(_), None) => false,
        (None, _) => true,
    }
}

/// Fields required to create a [`ProtocolSession`].
///
/// Grouped into a parameter struct to keep the constructor readable and avoid a
/// long positional argument list.
#[derive(Clone, Debug)]
pub struct NewProtocolSession {
    /// Session identity (UUIDv7).
    pub protocol_session_id: ProtocolSessionId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Device this session belongs to.
    pub device_id: DeviceId,
    /// Protocol of the session.
    pub protocol: Protocol,
    /// External protocol identity of the device (e.g. its GB device id).
    pub protocol_identity: ProtocolIdentity,
    /// Local listener identity.
    pub local_identity: LocalIdentity,
    /// SIP transport.
    pub transport: SipTransport,
    /// Endpoint routing facts.
    pub endpoint: SessionEndpoint,
    /// REGISTER transaction facts.
    pub registration: RegistrationInfo,
    /// Absolute time at which the registration expires.
    pub expiry_at: UtcTimestamp,
    /// Owner node currently holding the session, if any.
    pub owner_node_id: Option<NodeId>,
    /// Owner epoch used to fence stale nodes.
    pub owner_epoch: OwnerEpoch,
    /// Compatibility profile.
    pub compatibility: CompatibilityProfile,
}

/// Persistent GB28181 protocol session aggregate.
///
/// All fields are private and can only change through methods that preserve the
/// aggregate invariants and bump the optimistic-concurrency [`Revision`].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProtocolSession {
    protocol_session_id: ProtocolSessionId,
    tenant_id: TenantId,
    device_id: DeviceId,
    protocol: Protocol,
    protocol_identity: ProtocolIdentity,
    local_identity: LocalIdentity,
    transport: SipTransport,
    endpoint: SessionEndpoint,
    registration: RegistrationInfo,
    expiry_at: UtcTimestamp,
    last_authenticated_at: Option<UtcTimestamp>,
    presence: PresenceState,
    last_keepalive_at: Option<UtcTimestamp>,
    offline_reason: Option<String>,
    owner_node_id: Option<NodeId>,
    owner_epoch: OwnerEpoch,
    compatibility: CompatibilityProfile,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl ProtocolSession {
    /// Creates a new protocol session.
    pub fn new(clock: &dyn Clock, params: NewProtocolSession) -> crate::Result<Self> {
        if params.protocol_session_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "protocol_session_id must not be nil",
            ));
        }
        if params.protocol == Protocol::Unknown {
            return Err(DomainError::invalid_argument("protocol must be known"));
        }
        if params.protocol_identity.as_str().is_empty() {
            return Err(DomainError::invalid_argument(
                "protocol_identity must not be empty",
            ));
        }
        params.local_identity.validate()?;
        params.endpoint.validate()?;
        params.registration.validate()?;
        params.compatibility.validate()?;

        let now = clock.now_wall();
        Ok(Self {
            protocol_session_id: params.protocol_session_id,
            tenant_id: params.tenant_id,
            device_id: params.device_id,
            protocol: params.protocol,
            protocol_identity: params.protocol_identity,
            local_identity: params.local_identity,
            transport: params.transport,
            endpoint: params.endpoint,
            registration: params.registration,
            expiry_at: params.expiry_at,
            last_authenticated_at: Some(now),
            presence: PresenceState::Unknown,
            last_keepalive_at: None,
            offline_reason: None,
            owner_node_id: params.owner_node_id,
            owner_epoch: params.owner_epoch,
            compatibility: params.compatibility,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        })
    }

    /// Refreshes the registration after an authenticated REGISTER renewal.
    ///
    /// Advances the CSeq/expiry and records the authentication time. The
    /// endpoint is only updated when `endpoint` is supplied, matching the rule
    /// that plain keepalives must not rewrite the route.
    pub fn refresh_registration(
        &mut self,
        clock: &dyn Clock,
        registration: RegistrationInfo,
        expiry_at: UtcTimestamp,
        endpoint: Option<SessionEndpoint>,
    ) -> crate::Result<()> {
        registration.validate()?;
        if registration.cseq < self.registration.cseq {
            return Err(DomainError::invalid_argument(
                "REGISTER CSeq must not decrease",
            ));
        }
        if let Some(endpoint) = endpoint {
            endpoint.validate()?;
            self.endpoint = endpoint;
        }
        self.registration = registration;
        self.expiry_at = expiry_at;
        let now = clock.now_wall();
        self.last_authenticated_at = Some(now);
        self.presence = PresenceState::Online;
        self.offline_reason = None;
        self.bump(clock);
        Ok(())
    }

    /// Records a keepalive that kept the device online.
    pub fn record_keepalive(&mut self, clock: &dyn Clock) {
        let now = clock.now_wall();
        self.last_keepalive_at = Some(now);
        self.presence = PresenceState::Online;
        self.offline_reason = None;
        self.bump(clock);
    }

    /// Marks the session's device as offline with a diagnostic reason.
    pub fn mark_offline(
        &mut self,
        clock: &dyn Clock,
        reason: impl Into<String>,
    ) -> crate::Result<()> {
        let reason = reason.into();
        if reason.len() > MAX_FIELD_BYTES {
            return Err(DomainError::invalid_argument("offline_reason too long"));
        }
        self.presence = PresenceState::Offline;
        self.offline_reason = Some(reason);
        self.bump(clock);
        Ok(())
    }

    /// Assigns ownership to a node, incrementing the owner epoch for fencing.
    pub fn assign_owner(&mut self, clock: &dyn Clock, node_id: NodeId, owner_epoch: OwnerEpoch) {
        self.owner_node_id = Some(node_id);
        self.owner_epoch = owner_epoch;
        self.bump(clock);
    }

    /// Returns `true` if the registration has expired at `now`.
    pub fn is_expired(&self, now: UtcTimestamp) -> bool {
        self.expiry_at <= now
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    /// Session identifier.
    pub fn protocol_session_id(&self) -> ProtocolSessionId {
        self.protocol_session_id
    }

    /// Owning tenant.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Device identifier.
    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Protocol.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// External protocol identity.
    pub fn protocol_identity(&self) -> &ProtocolIdentity {
        &self.protocol_identity
    }

    /// Local listener identity.
    pub fn local_identity(&self) -> &LocalIdentity {
        &self.local_identity
    }

    /// SIP transport.
    pub fn transport(&self) -> SipTransport {
        self.transport
    }

    /// Endpoint routing facts.
    pub fn endpoint(&self) -> &SessionEndpoint {
        &self.endpoint
    }

    /// Registration transaction facts.
    pub fn registration(&self) -> &RegistrationInfo {
        &self.registration
    }

    /// Absolute registration expiry time.
    pub fn expiry_at(&self) -> UtcTimestamp {
        self.expiry_at
    }

    /// Time of the last successful authentication, if any.
    pub fn last_authenticated_at(&self) -> Option<UtcTimestamp> {
        self.last_authenticated_at
    }

    /// Presence state.
    pub fn presence(&self) -> PresenceState {
        self.presence
    }

    /// Time of the last keepalive, if any.
    pub fn last_keepalive_at(&self) -> Option<UtcTimestamp> {
        self.last_keepalive_at
    }

    /// Reason the device is offline, if any.
    pub fn offline_reason(&self) -> Option<&str> {
        self.offline_reason.as_deref()
    }

    /// Owner node holding the session, if any.
    pub fn owner_node_id(&self) -> Option<NodeId> {
        self.owner_node_id
    }

    /// Owner epoch used for fencing.
    pub fn owner_epoch(&self) -> OwnerEpoch {
        self.owner_epoch
    }

    /// Compatibility profile.
    pub fn compatibility(&self) -> &CompatibilityProfile {
        &self.compatibility
    }

    /// Creation time.
    pub fn created_at(&self) -> UtcTimestamp {
        self.created_at
    }

    /// Last update time.
    pub fn updated_at(&self) -> UtcTimestamp {
        self.updated_at
    }

    /// Optimistic concurrency revision.
    pub fn revision(&self) -> Revision {
        self.revision
    }
}
