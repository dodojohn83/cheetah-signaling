//! Sans-I/O GB28181 cascade upstream registration state machine.

mod machine;
mod registration;

use crate::cascade::registration::validate_token;
use crate::events::Gb28181Event;
use crate::types::DomainId;
use cheetah_gb28181_core::{DigestChallenge, DigestClient, DigestError, SipMessage, SipUri};
use cheetah_signal_types::is_internal_ip;
use secrecy::SecretString;
use std::net::IpAddr;
use std::str::FromStr;

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
    /// Optional `User-Agent` header value.
    pub user_agent: Option<String>,
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

        if let Ok(ip) = IpAddr::from_str(upstream.host())
            && is_internal_ip(ip)
            && !allow_internal_upstreams
        {
            return Err(CascadeError::Internal(
                "upstream host is an internal IP".to_string(),
            ));
        }

        Ok(Self {
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
            user_agent: None,
        })
    }
}

/// An event delivered to the cascade state machine.
#[derive(Clone, Debug)]
pub enum CascadeEvent {
    /// Start or restart the upstream registration.
    Register,
    /// Unregister from the upstream platform.
    Deregister,
    /// A SIP response received from the network.
    Response(Box<SipMessage>),
    /// A periodic tick for refresh, retry and transaction timeout processing.
    Tick,
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

#[derive(Clone, Debug)]
struct Registered {
    cseq: u32,
    call_id: String,
    local_tag: String,
    refresh_at: u64,
    challenge: Option<DigestChallenge>,
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

/// Sans-I/O state machine for an upstream GB28181 cascade platform.
#[derive(Clone, Debug)]
pub struct Gb28181Cascade<P: CascadeCredentialProvider> {
    config: CascadeConfig,
    provider: P,
    state: State,
    request_counter: u64,
    auth: Option<AuthorizationContext>,
}

#[cfg(test)]
mod tests;
