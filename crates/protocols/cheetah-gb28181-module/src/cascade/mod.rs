//! Sans-I/O GB28181 cascade upstream registration state machine.

mod bridge;
mod catalog;
mod keepalive;
mod machine;
mod machine_response;
mod manager;
mod registration;
mod report;
mod request_handler;
mod subscription;

pub use catalog::{CatalogError, CatalogFilter, CatalogPage, CatalogProvider, CatalogQuery};
pub use manager::{CascadeManager, CascadeRoutingError};

use crate::endpoint_policy::{EndpointPolicy, require_explicit_advertised_host};
use crate::events::Gb28181Event;
use crate::types::DomainId;
use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, DigestContext, DigestError, DigestReplayCache, SipMessage,
    SipUri,
};
use secrecy::{SecretBox, SecretString};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Builds the network-zone policy applied to the cascade upstream endpoint.
///
/// The upstream is an outbound target crossing a trust boundary, so by default
/// only public `sip`/`sips` endpoints are accepted. When
/// `allow_internal_upstreams` is set (private / 专网 deployments) every
/// non-unspecified zone is admitted, but the unspecified address is still
/// rejected because it is never a valid registrar.
pub(crate) fn upstream_endpoint_policy(allow_internal: bool) -> EndpointPolicy {
    if allow_internal {
        EndpointPolicy::any_zone_sip()
    } else {
        EndpointPolicy::public_sip()
    }
}

/// Provider for upstream platform credentials.
pub trait CascadeCredentialProvider: Send + Sync {
    /// Returns the password for the credential reference configured for an
    /// upstream platform.
    fn password_for(&self, credential_ref: &str) -> Option<SecretString>;
}

impl<F> CascadeCredentialProvider for F
where
    F: Fn(&str) -> Option<SecretString> + Send + Sync,
{
    fn password_for(&self, credential_ref: &str) -> Option<SecretString> {
        (self)(credential_ref)
    }
}

/// Rejects values that would inject extra SIP header lines.
pub(crate) fn validate_token(value: &str) -> Result<(), CascadeError> {
    if value.contains('\r') || value.contains('\n') {
        return Err(CascadeError::Internal(
            "SIP header token contains forbidden line break".to_string(),
        ));
    }
    Ok(())
}

/// Maximum number of catalog items a single SIP MESSAGE can carry.
const MAX_CATALOG_ITEMS_PER_PACKET: u32 = 10_000;
/// Maximum number of catalog response pages emitted for one upstream query.
const MAX_CATALOG_QUERY_PAGES: u32 = 10_000;
/// Maximum number of concurrent upstream play bridge sessions.
const MAX_MEDIA_BRIDGE_SESSIONS: u32 = 10_000;
/// Maximum number of concurrent upstream subscriptions.
const MAX_SUBSCRIPTIONS: u32 = 10_000;
/// Maximum number of pending upstream event reports to queue.
const MAX_REPORT_QUEUE_SIZE: u32 = 10_000;
/// Maximum consecutive failed REGISTER attempts before disconnecting.
const MAX_RETRIES: u32 = 100;
/// Maximum backoff delay (1 hour).
const MAX_BACKOFF_MS: u64 = 3_600_000;
/// Maximum jitter added to backoff (1 minute).
const MAX_JITTER_MS: u64 = 60_000;
/// Maximum subscription expiry (30 days).
const MAX_SUBSCRIPTION_EXPIRY_SECONDS: u32 = 2_592_000;

/// Configuration for one upstream GB28181 cascade platform.
#[derive(Clone, Debug)]
pub struct CascadeConfig {
    /// Logical domain that owns this cascade relationship.
    pub domain_id: DomainId,
    /// Local platform SIP URI (the AOR registered with the upstream).
    pub local_uri: SipUri,
    /// Upstream registrar SIP URI.
    pub upstream: SipUri,
    /// Authentication realm advertised by the upstream.
    pub realm: String,
    /// Opaque reference passed to the credential provider to obtain the
    /// upstream password.
    pub credential_ref: String,
    /// Desired registration lifetime in seconds.
    pub register_interval_seconds: u32,
    /// How many seconds before expiry to schedule the next refresh. The actual
    /// margin is clamped to at most half of the server-granted expiry to avoid
    /// a refresh loop when the upstream returns a short lifetime.
    pub register_refresh_margin_seconds: u32,
    /// Maximum number of consecutive failed REGISTER attempts before emitting
    /// a disconnection event.
    pub max_retries: u32,
    /// Initial backoff delay in milliseconds after a failed attempt.
    pub base_backoff_ms: u64,
    /// Maximum backoff delay in milliseconds.
    pub max_backoff_ms: u64,
    /// Maximum jitter added to backoff, in milliseconds.
    pub jitter_ms: u64,
    /// How long a single REGISTER transaction may stay pending before the
    /// state machine treats it as failed.
    pub transaction_timeout_seconds: u32,
    /// Whether to allow MD5 digest algorithm for legacy interop.
    pub allow_md5: bool,
    /// Whether internal IP literals are accepted as upstream targets.
    pub allow_internal_upstreams: bool,
    /// Optional network-zone policy applied to the connection addresses of SDP
    /// offers/answers exchanged with the upstream platform. When set, an
    /// upstream INVITE whose SDP advertises an address outside the policy is
    /// rejected with `400`. `None` (the default) accepts any parseable SDP,
    /// preserving legacy behaviour for private / 专网 deployments.
    pub sdp_endpoint_policy: Option<EndpointPolicy>,
    /// Interval in seconds between periodic keepalive MESSAGE requests.
    pub keepalive_interval_seconds: u32,
    /// How long a keepalive MESSAGE transaction may stay pending.
    pub keepalive_timeout_seconds: u32,
    /// Maximum consecutive keepalive failures before marking the platform
    /// disconnected.
    pub keepalive_max_failures: u32,
    /// Maximum number of catalog items per SIP packet.
    pub catalog_max_items_per_packet: u32,
    /// Maximum number of catalog response packets emitted for one upstream
    /// query. This bounds both memory and transaction state for large catalogs.
    pub catalog_max_query_pages: u32,
    /// Maximum number of concurrent upstream play bridge sessions.
    pub media_bridge_max_sessions: u32,
    /// How long an upstream INVITE transaction may stay in `Invited` or
    /// `Accepted` state before the bridge is abandoned and cleaned up.
    pub media_bridge_transaction_timeout_seconds: u32,
    /// How long an active bridge may stay without a `BYE` before the cascade
    /// sends its own `BYE`. A value of `0` disables the active timeout.
    pub media_bridge_active_timeout_seconds: u32,
    /// Filter controlling which resources may be shared with the upstream
    /// platform.
    pub catalog_filter: CatalogFilter,
    /// Optional `User-Agent` header value.
    pub user_agent: Option<String>,
    /// Optional credential reference used to look up the password for
    /// validating SIP Digest `Authorization` headers on incoming `Catalog`
    /// `MESSAGE` requests. If omitted, `credential_ref` is used as a fallback.
    pub catalog_inbound_digest_credential_ref: Option<String>,
    /// Optional HMAC server secret for generating nonces when challenging
    /// incoming `Catalog` `MESSAGE` requests with `401`. Must be at least 32
    /// bytes when provided. Wrapped in `Arc` so the secret is shared rather
    /// than copied when the configuration is cloned.
    pub catalog_inbound_digest_server_secret: Option<Arc<SecretBox<[u8]>>>,
    /// Maximum number of pending upstream event reports to queue while not
    /// registered or while the upstream is slow.
    pub report_max_queue_size: u32,
    /// Maximum number of concurrent upstream subscriptions.
    pub subscription_max_subscriptions: u32,
    /// Default subscription lifetime in seconds when the upstream omits Expires.
    pub subscription_default_expiry_seconds: u32,
    /// Minimum subscription lifetime the cascade is willing to grant.
    pub subscription_min_expiry_seconds: u32,
    /// Maximum subscription lifetime the cascade is willing to grant.
    pub subscription_max_expiry_seconds: u32,
}

impl CascadeConfig {
    /// Creates a validated cascade configuration.
    pub fn new(
        domain_id: DomainId,
        local_uri: SipUri,
        upstream: SipUri,
        realm: String,
        credential_ref: String,
        register_interval_seconds: u32,
    ) -> Result<Self, CascadeError> {
        Self::with_options(
            domain_id,
            local_uri,
            upstream,
            realm,
            credential_ref,
            register_interval_seconds,
            30,
            false,
            false,
        )
    }

    /// Creates a validated cascade configuration with legacy options.
    #[allow(clippy::too_many_arguments)]
    pub fn with_options(
        domain_id: DomainId,
        local_uri: SipUri,
        upstream: SipUri,
        realm: String,
        credential_ref: String,
        register_interval_seconds: u32,
        register_refresh_margin_seconds: u32,
        allow_md5: bool,
        allow_internal_upstreams: bool,
    ) -> Result<Self, CascadeError> {
        if register_interval_seconds == 0 {
            return Err(CascadeError::Internal(
                "register_interval_seconds must be greater than zero".to_string(),
            ));
        }
        if register_refresh_margin_seconds == 0 {
            return Err(CascadeError::Internal(
                "register_refresh_margin_seconds must be greater than zero".to_string(),
            ));
        }
        validate_token(&realm)?;
        validate_token(&credential_ref)?;
        validate_token(local_uri.user().unwrap_or(""))?;
        validate_token(upstream.host())?;

        // The advertised (local) platform address must be explicitly
        // configured, never derived from an untrusted Host/Contact header.
        require_explicit_advertised_host(local_uri.host()).map_err(|e| {
            CascadeError::Internal(format!("local advertised host is not explicit: {e}"))
        })?;

        // Validate the outbound upstream endpoint (scheme/transport/port and,
        // for IP-literal hosts, the network zone). Domain-name hosts defer to
        // DNS re-verification (see `verify_upstream_resolved_addresses`).
        upstream_endpoint_policy(allow_internal_upstreams)
            .validate_sip_endpoint(&upstream)
            .map_err(|e| CascadeError::Internal(format!("invalid upstream endpoint: {e}")))?;

        let mut config = Self {
            domain_id,
            local_uri,
            upstream,
            realm,
            credential_ref,
            register_interval_seconds,
            register_refresh_margin_seconds,
            max_retries: 5,
            base_backoff_ms: 1_000,
            max_backoff_ms: 60_000,
            jitter_ms: 1_000,
            transaction_timeout_seconds: 32,
            allow_md5,
            allow_internal_upstreams,
            sdp_endpoint_policy: None,
            keepalive_interval_seconds: 30,
            keepalive_timeout_seconds: 10,
            keepalive_max_failures: 3,
            catalog_max_items_per_packet: 100,
            catalog_max_query_pages: 1000,
            media_bridge_max_sessions: 1000,
            media_bridge_transaction_timeout_seconds: 60,
            // A generous safety deadline (7 days) so a stuck active bridge is
            // eventually reclaimed even if BYE/stop signals are lost. Set to 0
            // to disable active-timeout cleanup explicitly.
            media_bridge_active_timeout_seconds: 604_800,
            catalog_filter: CatalogFilter::default(),
            user_agent: None,
            catalog_inbound_digest_credential_ref: None,
            catalog_inbound_digest_server_secret: None,
            report_max_queue_size: 1000,
            subscription_max_subscriptions: 1000,
            subscription_default_expiry_seconds: 3600,
            subscription_min_expiry_seconds: 60,
            subscription_max_expiry_seconds: 86400,
        };
        config.sanitize();
        Ok(config)
    }

    /// Clamps all numeric limits to sensible production ceilings so a
    /// misconfigured or malicious cascade policy cannot allocate unbounded
    /// memory or retry forever.
    pub fn sanitize(&mut self) {
        self.catalog_max_items_per_packet = self
            .catalog_max_items_per_packet
            .clamp(1, MAX_CATALOG_ITEMS_PER_PACKET);
        self.catalog_max_query_pages = self
            .catalog_max_query_pages
            .clamp(1, MAX_CATALOG_QUERY_PAGES);
        self.media_bridge_max_sessions = self
            .media_bridge_max_sessions
            .clamp(0, MAX_MEDIA_BRIDGE_SESSIONS);
        self.subscription_max_subscriptions = self
            .subscription_max_subscriptions
            .clamp(0, MAX_SUBSCRIPTIONS);
        self.report_max_queue_size = self.report_max_queue_size.clamp(0, MAX_REPORT_QUEUE_SIZE);
        self.max_retries = self.max_retries.clamp(0, MAX_RETRIES);
        self.base_backoff_ms = self.base_backoff_ms.clamp(0, MAX_BACKOFF_MS);
        self.max_backoff_ms = self
            .max_backoff_ms
            .clamp(self.base_backoff_ms, MAX_BACKOFF_MS);
        self.jitter_ms = self.jitter_ms.clamp(0, MAX_JITTER_MS);
        self.subscription_default_expiry_seconds = self
            .subscription_default_expiry_seconds
            .clamp(0, MAX_SUBSCRIPTION_EXPIRY_SECONDS);
        self.subscription_min_expiry_seconds = self
            .subscription_min_expiry_seconds
            .clamp(0, MAX_SUBSCRIPTION_EXPIRY_SECONDS);
        self.subscription_max_expiry_seconds = self
            .subscription_max_expiry_seconds
            .clamp(0, MAX_SUBSCRIPTION_EXPIRY_SECONDS);
        // Ensure min/max expiry ordering is coherent even if a caller sets
        // inconsistent values after construction.
        let min_expiry = self
            .subscription_min_expiry_seconds
            .min(self.subscription_max_expiry_seconds);
        let max_expiry = self
            .subscription_min_expiry_seconds
            .max(self.subscription_max_expiry_seconds);
        self.subscription_min_expiry_seconds = min_expiry;
        self.subscription_max_expiry_seconds = max_expiry;
        self.subscription_default_expiry_seconds = self
            .subscription_default_expiry_seconds
            .clamp(min_expiry, max_expiry);
    }

    /// Enables SIP Digest authentication for incoming `Catalog` `MESSAGE`
    /// requests using the supplied credential reference and server secret.
    ///
    /// `server_secret` must be at least 32 bytes.
    pub fn with_catalog_inbound_digest(
        mut self,
        credential_ref: impl Into<String>,
        server_secret: impl AsRef<[u8]>,
    ) -> Result<Self, CascadeError> {
        let credential_ref = credential_ref.into();
        validate_token(&credential_ref)?;
        if server_secret.as_ref().len() < 32 {
            return Err(CascadeError::Internal(
                "catalog inbound digest server secret must be at least 32 bytes".to_string(),
            ));
        }
        let boxed: Box<[u8]> = Box::from(server_secret.as_ref());
        self.catalog_inbound_digest_credential_ref = Some(credential_ref);
        self.catalog_inbound_digest_server_secret = Some(Arc::new(SecretBox::new(boxed)));
        Ok(self)
    }

    /// Applies a network-zone policy to SDP connection addresses exchanged with
    /// the upstream platform.
    #[must_use]
    pub fn with_sdp_endpoint_policy(mut self, policy: EndpointPolicy) -> Self {
        self.sdp_endpoint_policy = Some(policy);
        self
    }

    /// Re-verifies the addresses the upstream host resolved to.
    ///
    /// The cascade state machine is Sans-I/O and never resolves DNS itself. A
    /// driver resolves [`Self::upstream`] and calls this both with the freshly
    /// resolved address set (before connecting) and with the connected peer
    /// address, so a name that resolved to a permitted address cannot later be
    /// rebound to an internal one. Every address must satisfy the same
    /// network-zone policy applied to IP-literal upstreams at construction.
    pub fn verify_upstream_resolved_addresses(
        &self,
        addresses: &[IpAddr],
    ) -> Result<(), CascadeError> {
        upstream_endpoint_policy(self.allow_internal_upstreams)
            .verify_resolved_addresses(addresses)
            .map_err(|e| CascadeError::Internal(format!("upstream address rejected: {e}")))
    }
}

/// An event delivered to the cascade state machine.
#[derive(Clone, Debug)]
pub enum CascadeEvent {
    /// Start or restart the upstream registration.
    Register,
    /// Unregister from the upstream platform.
    Deregister,
    /// A SIP request received from the network.
    Request(Box<SipMessage>),
    /// A SIP response received from the network.
    Response(Box<SipMessage>),
    /// The application has allocated media resources and produced an SDP answer
    /// for an upstream play bridge.
    BridgeMediaReady {
        /// Bridge identifier supplied by `CascadePlayRequested`.
        bridge_id: String,
        /// Answer SDP to send in the `200 OK` to the upstream platform.
        answer_sdp: String,
    },
    /// The application wants to tear down an upstream play bridge (e.g., the
    /// downstream device hung up or allocation failed).
    BridgeMediaStop {
        /// Bridge identifier supplied by `CascadePlayRequested`.
        bridge_id: String,
    },
    /// A periodic tick for refresh, retry and transaction timeout processing.
    Tick,
    /// A domain event that should be forwarded to the upstream platform
    /// according to the configured sharing policy.
    Report {
        /// The domain event produced by a device or another subsystem.
        event: Box<Gb28181Event>,
    },
}

/// A single input to the cascade state machine.
#[derive(Clone, Debug)]
pub struct CascadeInput {
    /// Monotonic second counter.
    pub now: u64,
    /// Event to process.
    pub event: CascadeEvent,
}

/// An output produced by the cascade state machine.
#[derive(Clone, Debug)]
pub enum CascadeOutput {
    /// A SIP request that the transport should send.
    SendRequest(SipMessage),
    /// A SIP response that the transport should send.
    SendResponse(SipMessage),
    /// A domain event for downstream consumers.
    EmitEvent(Gb28181Event),
}

/// Errors returned by the cascade state machine.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CascadeError {
    /// The cascade state machine is in an incompatible state for the input.
    #[error("invalid cascade state: {0}")]
    InvalidState(String),
    /// The SIP message cannot be used (missing Call-ID, malformed, etc.).
    #[error("malformed SIP message: {0}")]
    MalformedSip(String),
    /// The configured credential reference cannot be resolved.
    #[error("no credentials for upstream platform")]
    NoCredentials,
    /// The upstream authentication challenge is malformed or unsupported.
    #[error("authentication error: {0}")]
    AuthenticationFailed(String),
    /// An internal, non-recoverable module error.
    #[error("internal cascade error: {0}")]
    Internal(String),
}

impl From<DigestError> for CascadeError {
    fn from(e: DigestError) -> Self {
        CascadeError::AuthenticationFailed(e.to_string())
    }
}

impl From<cheetah_gb28181_core::SipError> for CascadeError {
    fn from(e: cheetah_gb28181_core::SipError) -> Self {
        CascadeError::MalformedSip(e.to_string())
    }
}

impl From<CatalogError> for CascadeError {
    fn from(e: CatalogError) -> Self {
        CascadeError::Internal(e.to_string())
    }
}

#[derive(Clone, Debug)]
struct Registered {
    cseq: u32,
    call_id: String,
    local_tag: String,
    refresh_at: u64,
    challenge: Option<DigestChallenge>,
    keepalive: Keepalive,
}

#[derive(Clone, Debug)]
struct Registering {
    cseq: u32,
    branch: String,
    call_id: String,
    local_tag: String,
    attempt: u32,
    started_at: u64,
    authenticated: bool,
    is_deregister: bool,
    previous: Option<Registered>,
    /// Cached challenge from the last 401 response; used to pre-authenticate
    /// refresh REGISTERs when the 200 OK does not repeat `WWW-Authenticate`.
    challenge: Option<DigestChallenge>,
}

/// Keepalive state tracked while the platform is registered.
#[derive(Clone, Debug)]
struct Keepalive {
    next_at: u64,
    pending_until: Option<u64>,
    failures: u32,
    sn: u32,
    call_id: String,
    cseq: u32,
}

impl Keepalive {
    fn new(next_at: u64, call_id: String, cseq: u32) -> Self {
        Self {
            next_at,
            pending_until: None,
            failures: 0,
            sn: 0,
            call_id,
            cseq,
        }
    }
}

#[derive(Clone, Debug)]
enum State {
    Idle,
    Registering(Registering),
    Registered(Registered),
    Failed { retry_at: u64, attempt: u32 },
    Deregistering(Registering),
}

/// Reusable digest authentication context for a single nonce.
#[derive(Clone, Debug)]
struct AuthorizationContext {
    challenge: DigestChallenge,
    client: DigestClient,
}

/// Server-side digest context for authenticating incoming `Catalog` `MESSAGE`
/// requests from the upstream platform.
struct InboundAuthContext {
    digest: DigestContext,
    replay: Mutex<DigestReplayCache>,
}

impl std::fmt::Debug for InboundAuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundAuthContext")
            .field("digest", &"[REDACTED]")
            .field("replay", &"[REDACTED]")
            .finish()
    }
}

/// Sans-I/O state machine for an upstream GB28181 cascade platform.
#[derive(Clone)]
pub struct Gb28181Cascade<P: CascadeCredentialProvider> {
    config: CascadeConfig,
    provider: P,
    state: State,
    request_counter: u64,
    bridge_counter: u64,
    report_counter: u64,
    auth: Option<AuthorizationContext>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    inbound_auth: Option<Arc<InboundAuthContext>>,
    bridges: BTreeMap<String, bridge::Bridge>,
    report_queue: std::collections::VecDeque<report::PendingReport>,
    report_state: std::collections::HashMap<String, report::PendingReport>,
    report_state_order: std::collections::VecDeque<String>,
    subscriptions: BTreeMap<String, subscription::Subscription>,
}

impl<P: CascadeCredentialProvider> std::fmt::Debug for Gb28181Cascade<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181Cascade")
            .field("config", &self.config)
            .field("state", &self.state)
            .field("request_counter", &self.request_counter)
            .field("bridge_counter", &self.bridge_counter)
            .field("bridges", &self.bridges.len())
            .field("auth", &self.auth.is_some())
            .field("catalog_provider", &self.catalog_provider.is_some())
            .field("inbound_auth", &self.inbound_auth.is_some())
            .field("subscriptions", &self.subscriptions.len())
            .finish()
    }
}

#[cfg(test)]
mod tests;
