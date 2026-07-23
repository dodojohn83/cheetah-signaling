//! GB28181 cascade platform link aggregate.
//!
//! A [`GbPlatformLink`] models a cascade relationship between this signaling
//! platform and a remote GB28181 platform. It captures the *control-plane*
//! facts a signaling node needs to drive the cascade state machine
//! (`GB4-CAS-002`..`006`) as durable, tenant-scoped, owner-fenced state:
//!
//! - the direction (register upward to an upstream, or accept a downstream);
//! - the local/remote [`ProtocolIdentity`] pair, realm/domain and transport
//!   endpoint;
//! - a credential *reference* (never a plaintext secret) and auth policy;
//! - the desired vs. actual registration state, Call-ID/CSeq, expiry and
//!   keepalive/backoff runtime;
//! - owner node/epoch and link generation used to fence stale nodes;
//! - the ACL that scopes which tenant resources, catalog prefixes and
//!   control/media capabilities are shared over the link;
//! - subscription limits and the pinned compatibility profile.
//!
//! The aggregate performs no I/O. The REGISTER / keepalive / catalog /
//! subscription / bridge transaction chains that consume this state live in the
//! `cheetah-gb28181-module` cascade state machine and the application layer.

use crate::{DomainError, SipTransport};
use cheetah_signal_types::{
    Clock, NodeId, OwnerEpoch, PlatformLinkId, ProtocolIdentity, Revision, TenantId, UtcTimestamp,
};

/// Maximum byte length of the free-form string fields carried by a link.
const MAX_FIELD_BYTES: usize = 512;
/// Maximum number of entries in an ACL collection.
const MAX_ACL_ENTRIES: usize = 256;
/// Hard ceiling on the number of concurrent cascade hops we will forward
/// through before treating the topology as a loop.
pub const MAX_CASCADE_HOPS: usize = 8;

/// Rejects values that could inject extra SIP header lines when the field is
/// later serialised into a request the cascade sends.
fn validate_token(name: &str, value: &str) -> crate::Result<()> {
    if value.contains('\r') || value.contains('\n') {
        return Err(DomainError::invalid_argument(format!(
            "{name} must not contain line breaks"
        )));
    }
    if value.len() > MAX_FIELD_BYTES {
        return Err(DomainError::invalid_argument(format!("{name} too long")));
    }
    Ok(())
}

/// Direction of a cascade platform link relative to this platform.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PlatformDirection {
    /// This platform registers *up* to a remote upstream platform.
    #[default]
    Upstream,
    /// A remote downstream platform registers *in* to this platform.
    Downstream,
}

impl std::fmt::Display for PlatformDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Upstream => "upstream",
            Self::Downstream => "downstream",
        })
    }
}

impl std::str::FromStr for PlatformDirection {
    type Err = DomainError;

    fn from_str(s: &str) -> crate::Result<Self> {
        let direction = if s.eq_ignore_ascii_case("upstream") {
            Self::Upstream
        } else if s.eq_ignore_ascii_case("downstream") {
            Self::Downstream
        } else {
            let display = s.chars().take(64).collect::<String>();
            return Err(DomainError::invalid_argument(format!(
                "unknown platform direction: {display}"
            )));
        };
        Ok(direction)
    }
}

/// Desired registration state for a link, set by the application.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DesiredRegistrationState {
    /// The link should be registered and kept alive.
    Registered,
    /// The link should be unregistered / torn down.
    #[default]
    Unregistered,
}

impl std::fmt::Display for DesiredRegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Registered => "registered",
            Self::Unregistered => "unregistered",
        })
    }
}

/// Actual registration state observed by the cascade state machine.
#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActualRegistrationState {
    /// No registration in progress.
    #[default]
    Idle,
    /// A REGISTER transaction is in flight.
    Registering,
    /// The link is registered and healthy.
    Registered,
    /// Registration failed; the link is backing off before the next attempt.
    Failed,
    /// A deregistration (Expires=0) transaction is in flight.
    Deregistering,
}

impl std::fmt::Display for ActualRegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Idle => "idle",
            Self::Registering => "registering",
            Self::Registered => "registered",
            Self::Failed => "failed",
            Self::Deregistering => "deregistering",
        })
    }
}

/// The local and remote platform protocol identities of a link.
///
/// These are the GB28181 platform SIP IDs (e.g. `34020000002000000001`). They
/// are external identities and must never be confused with internal device or
/// tenant UUIDs.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlatformIdentityPair {
    /// Identity this platform presents on the link (the local platform id).
    pub local: ProtocolIdentity,
    /// Identity of the remote platform.
    pub remote: ProtocolIdentity,
}

impl PlatformIdentityPair {
    fn validate(&self) -> crate::Result<()> {
        if self.local.as_str().is_empty() {
            return Err(DomainError::invalid_argument(
                "local platform identity must not be empty",
            ));
        }
        if self.remote.as_str().is_empty() {
            return Err(DomainError::invalid_argument(
                "remote platform identity must not be empty",
            ));
        }
        validate_token("local platform identity", self.local.as_str())?;
        validate_token("remote platform identity", self.remote.as_str())?;
        if self.local == self.remote {
            return Err(DomainError::invalid_argument(
                "local and remote platform identities must differ",
            ));
        }
        Ok(())
    }
}

/// Transport endpoint and SIP naming for the remote platform.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlatformEndpoint {
    /// Remote host (IP literal or resolvable name subject to outbound policy).
    pub host: String,
    /// Remote SIP port.
    pub port: u16,
    /// SIP transport.
    pub transport: SipTransport,
    /// Digest realm advertised on the link.
    pub realm: String,
    /// SIP domain of the link.
    pub domain: String,
}

impl PlatformEndpoint {
    fn validate(&self) -> crate::Result<()> {
        if self.host.is_empty() {
            return Err(DomainError::invalid_argument(
                "endpoint host must not be empty",
            ));
        }
        if self.port == 0 {
            return Err(DomainError::invalid_argument(
                "endpoint port must be non-zero",
            ));
        }
        validate_token("endpoint host", &self.host)?;
        validate_token("endpoint realm", &self.realm)?;
        validate_token("endpoint domain", &self.domain)?;
        Ok(())
    }
}

/// Reference to the credential used to authenticate the link.
///
/// Only the reference and non-secret policy are stored; the plaintext password
/// is resolved through the `SecretStore` at transaction time.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlatformCredential {
    /// Opaque reference passed to the secret store.
    pub credential_ref: String,
    /// Whether MD5 digest is permitted for legacy interop.
    pub allow_md5: bool,
}

impl PlatformCredential {
    fn validate(&self) -> crate::Result<()> {
        validate_token("credential_ref", &self.credential_ref)?;
        Ok(())
    }
}

/// Access-control policy scoping what a link may see and do.
///
/// Empty `allowed_catalog_prefixes` means *no* resources are shared until a
/// prefix is added; this is deliberately closed-by-default so a
/// misconfiguration cannot leak an entire tenant catalog.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlatformAcl {
    /// External-id prefixes the remote platform may enumerate/subscribe to.
    pub allowed_catalog_prefixes: Vec<String>,
    /// Whether the remote platform may issue control commands (PTZ, etc.).
    pub allow_control: bool,
    /// Whether the remote platform may negotiate media bridges.
    pub allow_media: bool,
    /// Remote platform identities that must never be bridged to (explicit
    /// loop / topology denials).
    pub denied_platform_ids: Vec<String>,
}

impl PlatformAcl {
    fn validate(&self) -> crate::Result<()> {
        if self.allowed_catalog_prefixes.len() > MAX_ACL_ENTRIES {
            return Err(DomainError::invalid_argument(
                "too many allowed catalog prefixes",
            ));
        }
        if self.denied_platform_ids.len() > MAX_ACL_ENTRIES {
            return Err(DomainError::invalid_argument(
                "too many denied platform ids",
            ));
        }
        for prefix in &self.allowed_catalog_prefixes {
            validate_token("catalog prefix", prefix)?;
        }
        for id in &self.denied_platform_ids {
            validate_token("denied platform id", id)?;
        }
        Ok(())
    }

    /// Returns `true` when `external_id` is covered by an allowed prefix.
    pub fn allows_resource(&self, external_id: &str) -> bool {
        self.allowed_catalog_prefixes
            .iter()
            .any(|prefix| external_id.starts_with(prefix.as_str()))
    }

    /// Returns `true` when `platform_id` is explicitly denied.
    pub fn is_denied_platform(&self, platform_id: &str) -> bool {
        self.denied_platform_ids.iter().any(|id| id == platform_id)
    }
}

/// Bounded exponential-backoff policy for registration retries.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BackoffPolicy {
    /// Initial backoff in milliseconds.
    pub base_ms: u64,
    /// Maximum backoff in milliseconds.
    pub max_ms: u64,
    /// Maximum jitter added to a backoff, in milliseconds.
    pub jitter_ms: u64,
    /// Consecutive failures before the link is reported disconnected.
    pub max_retries: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            base_ms: 1_000,
            max_ms: 60_000,
            jitter_ms: 1_000,
            max_retries: 5,
        }
    }
}

impl BackoffPolicy {
    fn validate(&self) -> crate::Result<()> {
        if self.base_ms == 0 {
            return Err(DomainError::invalid_argument(
                "backoff base_ms must be non-zero",
            ));
        }
        if self.max_ms < self.base_ms {
            return Err(DomainError::invalid_argument(
                "backoff max_ms must be >= base_ms",
            ));
        }
        Ok(())
    }

    /// Computes the *deterministic* backoff (without jitter) for `attempt`,
    /// clamped to `max_ms`. Jitter is added by the driver from an injected
    /// random source; keeping the base deterministic makes tests reproducible.
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        let shift = attempt.min(63);
        let multiplier = 1u64 << shift;
        self.base_ms.saturating_mul(multiplier).min(self.max_ms)
    }
}

/// Subscription capacity for a link.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SubscriptionLimits {
    /// Maximum number of concurrent subscriptions.
    pub max_subscriptions: u32,
    /// Current number of active subscriptions.
    pub active_subscriptions: u32,
}

impl Default for SubscriptionLimits {
    fn default() -> Self {
        Self {
            max_subscriptions: 1_000,
            active_subscriptions: 0,
        }
    }
}

/// Mutable runtime facts of the registration transaction chain.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RegistrationRuntime {
    /// Current REGISTER Call-ID (per link generation).
    pub call_id: String,
    /// Highest REGISTER CSeq used.
    pub cseq: u32,
    /// Absolute time the current registration expires, if registered.
    pub expiry_at: Option<UtcTimestamp>,
    /// Time of the last successful keepalive, if any.
    pub last_keepalive_at: Option<UtcTimestamp>,
    /// Consecutive failed REGISTER attempts since the last success.
    pub consecutive_failures: u32,
    /// Earliest time the next attempt may be made after a failure.
    pub retry_at: Option<UtcTimestamp>,
}

/// Fields required to create a [`GbPlatformLink`].
#[derive(Clone, Debug)]
pub struct NewPlatformLink {
    /// Link identity (UUIDv7).
    pub platform_link_id: PlatformLinkId,
    /// Owning tenant.
    pub tenant_id: TenantId,
    /// Direction of the link.
    pub direction: PlatformDirection,
    /// Local/remote platform identities.
    pub identity: PlatformIdentityPair,
    /// Transport endpoint and SIP naming.
    pub endpoint: PlatformEndpoint,
    /// Credential reference and auth policy.
    pub credential: PlatformCredential,
    /// Access-control policy.
    pub acl: PlatformAcl,
    /// Backoff policy for registration retries.
    pub backoff: BackoffPolicy,
    /// Subscription limits.
    pub subscription_limits: SubscriptionLimits,
    /// Requested `Expires` value in seconds.
    pub register_interval_secs: u32,
    /// Pinned compatibility profile id, if any.
    pub compatibility_profile_id: Option<String>,
    /// Pinned compatibility profile revision.
    pub compatibility_profile_revision: u32,
}

/// Persistent GB28181 cascade platform link aggregate.
///
/// All fields are private; mutations go through methods that preserve the
/// invariants and bump the optimistic-concurrency [`Revision`].
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GbPlatformLink {
    platform_link_id: PlatformLinkId,
    tenant_id: TenantId,
    direction: PlatformDirection,
    identity: PlatformIdentityPair,
    endpoint: PlatformEndpoint,
    credential: PlatformCredential,
    acl: PlatformAcl,
    backoff: BackoffPolicy,
    subscription_limits: SubscriptionLimits,
    register_interval_secs: u32,
    desired: DesiredRegistrationState,
    actual: ActualRegistrationState,
    runtime: RegistrationRuntime,
    compatibility_profile_id: Option<String>,
    compatibility_profile_revision: u32,
    owner_node_id: Option<NodeId>,
    owner_epoch: OwnerEpoch,
    generation: u64,
    created_at: UtcTimestamp,
    updated_at: UtcTimestamp,
    revision: Revision,
}

impl GbPlatformLink {
    /// Creates a new platform link aggregate.
    pub fn new(clock: &dyn Clock, params: NewPlatformLink) -> crate::Result<Self> {
        if params.platform_link_id.as_uuid().is_nil() {
            return Err(DomainError::invalid_argument(
                "platform_link_id must not be nil",
            ));
        }
        if params.register_interval_secs == 0 {
            return Err(DomainError::invalid_argument(
                "register_interval_secs must be non-zero",
            ));
        }
        params.identity.validate()?;
        params.endpoint.validate()?;
        params.credential.validate()?;
        params.acl.validate()?;
        params.backoff.validate()?;
        if let Some(id) = &params.compatibility_profile_id {
            validate_token("compatibility_profile_id", id)?;
        }

        let now = clock.now_wall();
        Ok(Self {
            platform_link_id: params.platform_link_id,
            tenant_id: params.tenant_id,
            direction: params.direction,
            identity: params.identity,
            endpoint: params.endpoint,
            credential: params.credential,
            acl: params.acl,
            backoff: params.backoff,
            subscription_limits: params.subscription_limits,
            register_interval_secs: params.register_interval_secs,
            desired: DesiredRegistrationState::Unregistered,
            actual: ActualRegistrationState::Idle,
            runtime: RegistrationRuntime::default(),
            compatibility_profile_id: params.compatibility_profile_id,
            compatibility_profile_revision: params.compatibility_profile_revision,
            owner_node_id: None,
            owner_epoch: OwnerEpoch::default(),
            generation: 0,
            created_at: now,
            updated_at: now,
            revision: Revision::default(),
        })
    }

    /// Sets the desired registration state.
    pub fn set_desired(&mut self, clock: &dyn Clock, desired: DesiredRegistrationState) {
        if self.desired == desired {
            return;
        }
        self.desired = desired;
        self.bump(clock);
    }

    /// Assigns ownership to a node, fencing stale nodes with a monotonic epoch.
    ///
    /// The new epoch must be strictly greater than the current one; a smaller
    /// or equal epoch is rejected as [`DomainError::StaleOwner`]. Taking
    /// ownership also bumps the link generation so callbacks addressed to the
    /// previous owner/generation can be discarded.
    pub fn assign_owner(
        &mut self,
        clock: &dyn Clock,
        node_id: NodeId,
        owner_epoch: OwnerEpoch,
    ) -> crate::Result<()> {
        if owner_epoch.0 <= self.owner_epoch.0 && self.owner_node_id.is_some() {
            return Err(DomainError::stale_owner(self.owner_epoch.0, owner_epoch.0));
        }
        self.owner_node_id = Some(node_id);
        self.owner_epoch = owner_epoch;
        self.generation += 1;
        // A new owner starts a fresh registration transaction chain.
        self.runtime = RegistrationRuntime::default();
        self.actual = ActualRegistrationState::Idle;
        self.bump(clock);
        Ok(())
    }

    /// Returns `true` when a callback from `(node_id, epoch, generation)` is
    /// current and may advance this link's state.
    pub fn is_current_owner(
        &self,
        node_id: NodeId,
        owner_epoch: OwnerEpoch,
        generation: u64,
    ) -> bool {
        self.owner_node_id == Some(node_id)
            && self.owner_epoch == owner_epoch
            && self.generation == generation
    }

    /// Records that a REGISTER transaction has started.
    pub fn record_registering(&mut self, clock: &dyn Clock, call_id: String, cseq: u32) {
        self.actual = ActualRegistrationState::Registering;
        self.runtime.call_id = call_id;
        self.runtime.cseq = cseq;
        self.bump(clock);
    }

    /// Records a successful REGISTER, clearing failure/backoff state.
    pub fn record_registered(
        &mut self,
        clock: &dyn Clock,
        cseq: u32,
        expiry_at: UtcTimestamp,
    ) -> crate::Result<()> {
        if cseq < self.runtime.cseq {
            return Err(DomainError::invalid_argument(
                "REGISTER CSeq must not decrease",
            ));
        }
        self.actual = ActualRegistrationState::Registered;
        self.runtime.cseq = cseq;
        self.runtime.expiry_at = Some(expiry_at);
        self.runtime.consecutive_failures = 0;
        self.runtime.retry_at = None;
        self.bump(clock);
        Ok(())
    }

    /// Records a failed REGISTER attempt and schedules the next retry.
    ///
    /// Returns `true` when `max_retries` has been exceeded and the link should
    /// be reported as disconnected.
    pub fn record_registration_failure(
        &mut self,
        clock: &dyn Clock,
        retry_at: UtcTimestamp,
    ) -> bool {
        self.actual = ActualRegistrationState::Failed;
        self.runtime.consecutive_failures = self.runtime.consecutive_failures.saturating_add(1);
        self.runtime.retry_at = Some(retry_at);
        self.bump(clock);
        self.runtime.consecutive_failures > self.backoff.max_retries
    }

    /// Records a keepalive that kept the link healthy.
    pub fn record_keepalive(&mut self, clock: &dyn Clock) {
        let now = clock.now_wall();
        self.runtime.last_keepalive_at = Some(now);
        self.bump(clock);
    }

    /// Marks the start of a deregistration transaction.
    pub fn record_deregistering(&mut self, clock: &dyn Clock) {
        self.actual = ActualRegistrationState::Deregistering;
        self.bump(clock);
    }

    /// Marks the link idle after deregistration or teardown.
    pub fn record_idle(&mut self, clock: &dyn Clock) {
        self.actual = ActualRegistrationState::Idle;
        self.runtime.expiry_at = None;
        self.bump(clock);
    }

    /// Attempts to reserve one subscription slot, honoring the capacity limit.
    pub fn reserve_subscription(&mut self, clock: &dyn Clock) -> crate::Result<()> {
        if self.subscription_limits.active_subscriptions
            >= self.subscription_limits.max_subscriptions
        {
            return Err(DomainError::unavailable("subscription capacity exhausted"));
        }
        self.subscription_limits.active_subscriptions += 1;
        self.bump(clock);
        Ok(())
    }

    /// Releases one subscription slot.
    pub fn release_subscription(&mut self, clock: &dyn Clock) {
        if self.subscription_limits.active_subscriptions > 0 {
            self.subscription_limits.active_subscriptions -= 1;
            self.bump(clock);
        }
    }

    /// Returns `true` when `now` is at or past the registration expiry.
    pub fn is_expired(&self, now: UtcTimestamp) -> bool {
        matches!(self.runtime.expiry_at, Some(expiry) if expiry <= now)
    }

    /// Returns `true` when a bridge/control request that would traverse the
    /// platforms in `via_platform_ids` (the already-visited cascade path) must
    /// be rejected to prevent a routing loop.
    ///
    /// A loop exists when the remote or local identity of this link already
    /// appears in the visited path, when the remote platform is explicitly
    /// denied by the ACL, or when the hop count exceeds [`MAX_CASCADE_HOPS`].
    pub fn would_loop(&self, via_platform_ids: &[&str]) -> bool {
        if via_platform_ids.len() >= MAX_CASCADE_HOPS {
            return true;
        }
        if self.acl.is_denied_platform(self.identity.remote.as_str()) {
            return true;
        }
        let remote = self.identity.remote.as_str();
        let local = self.identity.local.as_str();
        via_platform_ids
            .iter()
            .any(|hop| *hop == remote || *hop == local)
    }

    fn bump(&mut self, clock: &dyn Clock) {
        self.updated_at = clock.now_wall();
        self.revision.0 += 1;
    }

    // --- Accessors -------------------------------------------------------

    /// Link identifier.
    pub fn platform_link_id(&self) -> PlatformLinkId {
        self.platform_link_id
    }
    /// Owning tenant.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }
    /// Direction of the link.
    pub fn direction(&self) -> PlatformDirection {
        self.direction
    }
    /// Local/remote platform identities.
    pub fn identity(&self) -> &PlatformIdentityPair {
        &self.identity
    }
    /// Transport endpoint.
    pub fn endpoint(&self) -> &PlatformEndpoint {
        &self.endpoint
    }
    /// Credential reference.
    pub fn credential(&self) -> &PlatformCredential {
        &self.credential
    }
    /// Access-control policy.
    pub fn acl(&self) -> &PlatformAcl {
        &self.acl
    }
    /// Backoff policy.
    pub fn backoff(&self) -> &BackoffPolicy {
        &self.backoff
    }
    /// Subscription limits.
    pub fn subscription_limits(&self) -> SubscriptionLimits {
        self.subscription_limits
    }
    /// Requested registration interval in seconds.
    pub fn register_interval_secs(&self) -> u32 {
        self.register_interval_secs
    }
    /// Desired registration state.
    pub fn desired(&self) -> DesiredRegistrationState {
        self.desired
    }
    /// Actual registration state.
    pub fn actual(&self) -> ActualRegistrationState {
        self.actual
    }
    /// Registration runtime facts.
    pub fn runtime(&self) -> &RegistrationRuntime {
        &self.runtime
    }
    /// Pinned compatibility profile id.
    pub fn compatibility_profile_id(&self) -> Option<&str> {
        self.compatibility_profile_id.as_deref()
    }
    /// Pinned compatibility profile revision.
    pub fn compatibility_profile_revision(&self) -> u32 {
        self.compatibility_profile_revision
    }
    /// Owner node holding the link, if any.
    pub fn owner_node_id(&self) -> Option<NodeId> {
        self.owner_node_id
    }
    /// Owner epoch used for fencing.
    pub fn owner_epoch(&self) -> OwnerEpoch {
        self.owner_epoch
    }
    /// Current link generation.
    pub fn generation(&self) -> u64 {
        self.generation
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

/// Returns `true` when appending `next` to the already-visited cascade `path`
/// would create a loop or exceed [`MAX_CASCADE_HOPS`].
///
/// This is the transport-agnostic core of cascade loop detection used by both
/// the domain aggregate and the protocol module's bridge routing.
pub fn detect_loop(path: &[String], next: &str) -> bool {
    if path.len() >= MAX_CASCADE_HOPS {
        return true;
    }
    path.iter().any(|hop| hop == next)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::in_memory::{InMemoryClock, InMemoryIdGenerator};
    use cheetah_signal_types::{Clock, IdGenerator};

    fn link(clock: &dyn Clock, ids: &dyn IdGenerator) -> GbPlatformLink {
        GbPlatformLink::new(
            clock,
            NewPlatformLink {
                platform_link_id: ids.generate_platform_link_id(),
                tenant_id: ids.generate_tenant_id(),
                direction: PlatformDirection::Upstream,
                identity: PlatformIdentityPair {
                    local: ProtocolIdentity::new("34020000002000000001").unwrap(),
                    remote: ProtocolIdentity::new("11000000002000000001").unwrap(),
                },
                endpoint: PlatformEndpoint {
                    host: "203.0.113.9".to_string(),
                    port: 5060,
                    transport: SipTransport::Udp,
                    realm: "1100000000".to_string(),
                    domain: "1100000000".to_string(),
                },
                credential: PlatformCredential {
                    credential_ref: "secret://upstream".to_string(),
                    allow_md5: false,
                },
                acl: PlatformAcl {
                    allowed_catalog_prefixes: vec!["3402000000".to_string()],
                    allow_control: true,
                    allow_media: true,
                    denied_platform_ids: vec![],
                },
                backoff: BackoffPolicy::default(),
                subscription_limits: SubscriptionLimits {
                    max_subscriptions: 2,
                    active_subscriptions: 0,
                },
                register_interval_secs: 3600,
                compatibility_profile_id: None,
                compatibility_profile_revision: 0,
            },
        )
        .unwrap()
    }

    #[test]
    fn rejects_equal_local_and_remote_identity() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let same = ProtocolIdentity::new("34020000002000000001").unwrap();
        let result = GbPlatformLink::new(
            &clock,
            NewPlatformLink {
                platform_link_id: ids.generate_platform_link_id(),
                tenant_id: ids.generate_tenant_id(),
                direction: PlatformDirection::Upstream,
                identity: PlatformIdentityPair {
                    local: same.clone(),
                    remote: same,
                },
                endpoint: PlatformEndpoint {
                    host: "203.0.113.9".to_string(),
                    port: 5060,
                    transport: SipTransport::Udp,
                    realm: "r".to_string(),
                    domain: "d".to_string(),
                },
                credential: PlatformCredential::default(),
                acl: PlatformAcl::default(),
                backoff: BackoffPolicy::default(),
                subscription_limits: SubscriptionLimits::default(),
                register_interval_secs: 3600,
                compatibility_profile_id: None,
                compatibility_profile_revision: 0,
            },
        );
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }

    #[test]
    fn owner_epoch_fences_stale_nodes() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let mut l = link(&clock, &ids);
        let node = ids.generate_node_id();
        l.assign_owner(&clock, node, OwnerEpoch(1)).unwrap();
        assert_eq!(l.generation(), 1);
        let older = l.assign_owner(&clock, ids.generate_node_id(), OwnerEpoch(1));
        assert!(matches!(older, Err(DomainError::StaleOwner { .. })));
        l.assign_owner(&clock, ids.generate_node_id(), OwnerEpoch(2))
            .unwrap();
        assert_eq!(l.generation(), 2);
        assert!(!l.is_current_owner(node, OwnerEpoch(1), 1));
    }

    #[test]
    fn registration_failure_reports_disconnect_past_max_retries() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let mut l = link(&clock, &ids);
        let now = clock.now_wall();
        for _ in 0..l.backoff().max_retries {
            assert!(!l.record_registration_failure(&clock, now));
        }
        assert!(l.record_registration_failure(&clock, now));
    }

    #[test]
    fn subscription_capacity_is_bounded() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let mut l = link(&clock, &ids);
        l.reserve_subscription(&clock).unwrap();
        l.reserve_subscription(&clock).unwrap();
        assert!(matches!(
            l.reserve_subscription(&clock),
            Err(DomainError::Unavailable { .. })
        ));
        l.release_subscription(&clock);
        l.reserve_subscription(&clock).unwrap();
    }

    #[test]
    fn acl_scopes_resources() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let l = link(&clock, &ids);
        assert!(l.acl().allows_resource("34020000001320000001"));
        assert!(!l.acl().allows_resource("99990000001320000001"));
    }

    #[test]
    fn loop_detection_flags_revisited_and_deep_paths() {
        let clock = InMemoryClock::new();
        let ids = InMemoryIdGenerator::new();
        let l = link(&clock, &ids);
        assert!(l.would_loop(&["11000000002000000001"]));
        assert!(!l.would_loop(&["55550000002000000001"]));
        let deep: Vec<String> = (0..MAX_CASCADE_HOPS).map(|i| format!("p{i}")).collect();
        let refs: Vec<&str> = deep.iter().map(String::as_str).collect();
        assert!(l.would_loop(&refs));
    }

    #[test]
    fn detect_loop_helper_matches_path_and_depth() {
        let path = vec!["a".to_string(), "b".to_string()];
        assert!(detect_loop(&path, "a"));
        assert!(!detect_loop(&path, "c"));
        let deep: Vec<String> = (0..MAX_CASCADE_HOPS).map(|i| i.to_string()).collect();
        assert!(detect_loop(&deep, "new"));
    }

    #[test]
    fn backoff_is_bounded_and_deterministic() {
        let policy = BackoffPolicy {
            base_ms: 1_000,
            max_ms: 8_000,
            jitter_ms: 0,
            max_retries: 5,
        };
        assert_eq!(policy.backoff_ms(0), 1_000);
        assert_eq!(policy.backoff_ms(1), 2_000);
        assert_eq!(policy.backoff_ms(3), 8_000);
        assert_eq!(policy.backoff_ms(30), 8_000);
        // Large attempts whose shifted base would overflow u64 must still
        // clamp to max_ms rather than wrapping to a small value.
        assert_eq!(policy.backoff_ms(55), 8_000);
        assert_eq!(policy.backoff_ms(63), 8_000);
        assert_eq!(policy.backoff_ms(u32::MAX), 8_000);
    }

    #[test]
    fn platform_direction_from_str_is_case_insensitive_and_bounds_error() {
        let parsed = "DOWNSTREAM".parse::<PlatformDirection>();
        assert!(matches!(parsed, Ok(PlatformDirection::Downstream)));

        let result = "x".repeat(1024).parse::<PlatformDirection>();
        assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    }
}
