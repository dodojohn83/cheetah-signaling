//! Sans-I/O GB28181 cascade upstream registration state machine.

mod registration;

use crate::cascade::registration::{build_register_request, validate_token};
use crate::events::Gb28181Event;
use crate::types::DomainId;
use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, DigestError, DigestResponse, HeaderName, Method, SipMessage,
    SipUri,
};
use cheetah_signal_types::is_internal_ip;
use secrecy::SecretString;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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
            false,
        )
    }

    /// Creates a validated cascade configuration with legacy options.
    pub fn with_options(
        domain_id: DomainId,
        local_uri: SipUri,
        upstream: SipUri,
        realm: String,
        credential_ref: String,
        register_interval_seconds: u32,
        allow_md5: bool,
    ) -> Result<Self, CascadeError> {
        if register_interval_seconds == 0 {
            return Err(CascadeError::Internal(
                "register_interval_seconds must be greater than zero".to_string(),
            ));
        }
        validate_token(&realm)?;
        validate_token(&credential_ref)?;
        validate_token(local_uri.user().unwrap_or(""))?;
        validate_token(upstream.host())?;

        if let Ok(ip) = IpAddr::from_str(upstream.host())
            && is_internal_ip(ip)
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
            max_retries: 5,
            base_backoff_ms: 1_000,
            max_backoff_ms: 60_000,
            jitter_ms: 1_000,
            transaction_timeout_seconds: 32,
            allow_md5,
            allow_internal_upstreams: false,
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
}

#[derive(Clone, Debug)]
enum State {
    Idle,
    Registering(Registering),
    Registered(Registered),
    Failed { retry_at: u64, attempt: u32 },
    Deregistering(Registering),
}

/// Sans-I/O state machine for an upstream GB28181 cascade platform.
#[derive(Clone, Debug)]
pub struct Gb28181Cascade<P: CascadeCredentialProvider> {
    config: CascadeConfig,
    provider: P,
    state: State,
    request_counter: u64,
}

impl<P: CascadeCredentialProvider> Gb28181Cascade<P> {
    /// Creates a new cascade state machine.
    pub fn new(config: CascadeConfig, provider: P) -> Self {
        Self {
            config,
            provider,
            state: State::Idle,
            request_counter: 0,
        }
    }

    /// Processes a single input and returns ordered outputs.
    pub fn process(&mut self, input: CascadeInput) -> Result<Vec<CascadeOutput>, CascadeError> {
        match input.event {
            CascadeEvent::Register => self.on_register(input.now),
            CascadeEvent::Deregister => self.on_deregister(input.now),
            CascadeEvent::Response(msg) => self.on_response(input.now, *msg),
            CascadeEvent::Tick => self.on_tick(input.now),
        }
    }

    fn on_register(&mut self, now: u64) -> Result<Vec<CascadeOutput>, CascadeError> {
        match &self.state {
            State::Idle | State::Failed { .. } => {
                let reg = self.start_registering(now, false, None)?;
                let msg = build_register_request(
                    &self.config,
                    &reg.call_id,
                    &reg.local_tag,
                    reg.cseq,
                    &reg.branch,
                    self.config.register_interval_seconds,
                    None,
                )?;
                self.state = State::Registering(reg);
                Ok(vec![CascadeOutput::SendRequest(msg)])
            }
            State::Registered(prev) => {
                let prev = prev.clone();
                let reg = self.start_registering(now, false, Some(prev.clone()))?;
                let auth = prev.challenge.as_ref().and_then(|challenge| {
                    self.build_authorization(challenge, reg.cseq, reg.attempt, now)
                        .ok()
                });
                let msg = build_register_request(
                    &self.config,
                    &reg.call_id,
                    &reg.local_tag,
                    reg.cseq,
                    &reg.branch,
                    self.config.register_interval_seconds,
                    auth.as_ref(),
                )?;
                self.state = State::Registering(reg);
                Ok(vec![CascadeOutput::SendRequest(msg)])
            }
            State::Registering(_) | State::Deregistering(_) => {
                // Already attempting registration; ignore duplicate command.
                Ok(Vec::new())
            }
        }
    }

    fn on_deregister(&mut self, now: u64) -> Result<Vec<CascadeOutput>, CascadeError> {
        match &self.state {
            State::Registered(prev) => {
                let prev = prev.clone();
                let mut reg = self.start_registering(now, true, Some(prev.clone()))?;
                let auth = prev.challenge.as_ref().and_then(|challenge| {
                    self.build_authorization(challenge, reg.cseq, reg.attempt, now)
                        .ok()
                });
                reg.is_deregister = true;
                let msg = build_register_request(
                    &self.config,
                    &reg.call_id,
                    &reg.local_tag,
                    reg.cseq,
                    &reg.branch,
                    0,
                    auth.as_ref(),
                )?;
                self.state = State::Deregistering(reg);
                Ok(vec![CascadeOutput::SendRequest(msg)])
            }
            State::Idle | State::Failed { .. } => {
                // Nothing to deregister.
                Ok(Vec::new())
            }
            State::Registering(_) | State::Deregistering(_) => {
                // Wait for the active transaction to complete.
                Ok(Vec::new())
            }
        }
    }

    fn on_tick(&mut self, now: u64) -> Result<Vec<CascadeOutput>, CascadeError> {
        match &self.state {
            State::Registered(reg) if now >= reg.refresh_at => {
                // Trigger a refresh.
                self.on_register(now)
            }
            State::Failed { retry_at, .. } if now >= *retry_at => {
                // Retry after backoff.
                self.on_register(now)
            }
            State::Registering(reg) | State::Deregistering(reg)
                if now
                    >= reg
                        .started_at
                        .saturating_add(self.config.transaction_timeout_seconds as u64) =>
            {
                // Transaction timed out; treat as a failed attempt.
                let attempt = reg.attempt;
                let is_deregister = reg.is_deregister;
                let reason = "REGISTER transaction timed out".to_string();
                Ok(self.fail_or_retry(now, attempt, is_deregister, reason))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn on_response(
        &mut self,
        now: u64,
        msg: SipMessage,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let (_cseq_num, cseq_method, call_id) = match &msg {
            SipMessage::Response { .. } => {
                let cseq = msg
                    .cseq()
                    .ok_or_else(|| CascadeError::MalformedSip("missing CSeq header".to_string()))?;
                let call_id = msg.call_id().ok_or_else(|| {
                    CascadeError::MalformedSip("missing Call-ID header".to_string())
                })?;
                (cseq.0, cseq.1, call_id.to_string())
            }
            SipMessage::Request { .. } => return Ok(Vec::new()),
        };

        if cseq_method != Method::Register {
            return Ok(Vec::new());
        }

        match &self.state {
            State::Registering(reg) if reg.call_id == call_id => {
                let reg = reg.clone();
                self.handle_register_response(now, msg, reg)
            }
            State::Deregistering(reg) if reg.call_id == call_id => {
                let reg = reg.clone();
                self.handle_deregister_response(now, msg, reg)
            }
            _ => Ok(Vec::new()),
        }
    }

    fn handle_register_response(
        &mut self,
        now: u64,
        msg: SipMessage,
        reg: Registering,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let status = match &msg {
            SipMessage::Response { line, .. } => line.clone(),
            _ => unreachable!("caller ensures a response"),
        };

        if status.code == 401 && !reg.authenticated {
            return self.challenge_and_resend(now, msg, reg);
        }

        if status.code == 401 && reg.authenticated {
            // Check if the nonce is stale; otherwise the password is wrong.
            if let Some(challenge) = extract_challenge(&msg)?
                && challenge.stale
            {
                return self.challenge_and_resend(now, msg, reg);
            }
            let reason = "authentication rejected by upstream".to_string();
            return Ok(self.fail_or_retry(now, reg.attempt, false, reason));
        }

        if status.code >= 400 {
            let reason = format!("REGISTER failed with {} {}", status.code, status.reason);
            return Ok(self.fail_or_retry(now, reg.attempt, false, reason));
        }

        if status.code >= 200 {
            let expires = parse_expires(&msg, self.config.register_interval_seconds);
            let challenge = extract_challenge(&msg).ok().flatten();

            let registered = Registered {
                cseq: reg.cseq,
                call_id: reg.call_id.clone(),
                local_tag: reg.local_tag.clone(),
                refresh_at: now.saturating_add(expires.saturating_sub(30).into()),
                challenge,
            };

            self.state = State::Registered(registered);
            return Ok(vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformConnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    upstream: self.config.upstream.encode(),
                    expires,
                },
            )]);
        }

        // Provisional response; wait for final.
        Ok(Vec::new())
    }

    fn handle_deregister_response(
        &mut self,
        now: u64,
        msg: SipMessage,
        reg: Registering,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let status = match &msg {
            SipMessage::Response { line, .. } => line.clone(),
            _ => unreachable!("caller ensures a response"),
        };

        if status.code == 401 && !reg.authenticated {
            return self.challenge_and_resend(now, msg, reg);
        }

        if status.code == 401 && reg.authenticated {
            // If already authenticated and still 401, give up on deregister.
            self.state = State::Idle;
            return Ok(vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason: "deregister rejected: authentication failed".to_string(),
                },
            )]);
        }

        if status.code >= 400 {
            // Deregister attempt failed, but the binding may still expire.
            self.state = State::Idle;
            return Ok(vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason: format!("deregister failed with {} {}", status.code, status.reason),
                },
            )]);
        }

        if status.code >= 200 {
            self.state = State::Idle;
            return Ok(vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason: "deregistered".to_string(),
                },
            )]);
        }

        Ok(Vec::new())
    }

    fn challenge_and_resend(
        &mut self,
        now: u64,
        msg: SipMessage,
        mut reg: Registering,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let challenge = extract_challenge(&msg)?.ok_or_else(|| {
            CascadeError::MalformedSip("401 response missing WWW-Authenticate".to_string())
        })?;

        let next_cseq = reg
            .cseq
            .checked_add(1)
            .ok_or_else(|| CascadeError::Internal("CSeq overflow".to_string()))?;
        let next_branch = self.next_branch(&reg.call_id, next_cseq);
        let next_attempt = reg.attempt + 1;
        let auth = self.build_authorization(&challenge, next_cseq, next_attempt, now)?;

        reg.cseq = next_cseq;
        reg.branch = next_branch;
        reg.attempt = next_attempt;
        reg.authenticated = true;
        reg.started_at = now;

        let expires = if reg.is_deregister {
            0
        } else {
            self.config.register_interval_seconds
        };
        let request = build_register_request(
            &self.config,
            &reg.call_id,
            &reg.local_tag,
            reg.cseq,
            &reg.branch,
            expires,
            Some(&auth),
        )?;

        // Store the challenge used for the next refresh/deregister.
        if !reg.is_deregister {
            reg.previous = reg.previous.take().map(|mut p| {
                p.challenge = Some(challenge);
                p
            });
        }

        self.state = if reg.is_deregister {
            State::Deregistering(reg)
        } else {
            State::Registering(reg)
        };

        Ok(vec![CascadeOutput::SendRequest(request)])
    }

    fn build_authorization(
        &self,
        challenge: &DigestChallenge,
        cseq: u32,
        attempt: u32,
        now: u64,
    ) -> Result<DigestResponse, CascadeError> {
        if challenge.qop == Some(cheetah_gb28181_core::DigestQop::AuthInt) {
            return Err(CascadeError::AuthenticationFailed(
                "auth-int qop is not supported".to_string(),
            ));
        }
        let password = self
            .provider
            .password_for(&self.config.credential_ref)
            .ok_or(CascadeError::NoCredentials)?;
        let cnonce = format!("{}-{cseq}-{attempt}-{now}", self.platform_id());
        let method = "REGISTER";
        let uri = self.config.upstream.encode();
        let mut client = DigestClient::new()
            .allow_md5(self.config.allow_md5)
            .qop(challenge.qop)?;
        let username = self.config.local_uri.user().unwrap_or("");
        Ok(client.authorize(username, &password, method, &uri, challenge, &cnonce)?)
    }

    fn start_registering(
        &mut self,
        now: u64,
        is_deregister: bool,
        previous: Option<Registered>,
    ) -> Result<Registering, CascadeError> {
        let cseq = match previous.as_ref() {
            Some(p) => p
                .cseq
                .checked_add(1)
                .ok_or_else(|| CascadeError::Internal("CSeq overflow".to_string()))?,
            None => 1,
        };
        let call_id = match previous.as_ref() {
            Some(p) => p.call_id.clone(),
            None => self.next_call_id(now),
        };
        let local_tag = match previous.as_ref() {
            Some(p) => p.local_tag.clone(),
            None => self.next_local_tag(now),
        };
        let branch = self.next_branch(&call_id, cseq);
        let attempt = match &self.state {
            State::Failed { attempt, .. } => *attempt,
            _ => 0,
        };

        Ok(Registering {
            cseq,
            branch,
            call_id,
            local_tag,
            attempt,
            started_at: now,
            authenticated: false,
            is_deregister,
            previous,
        })
    }

    fn fail_or_retry(
        &mut self,
        now: u64,
        attempt: u32,
        is_deregister: bool,
        reason: String,
    ) -> Vec<CascadeOutput> {
        if is_deregister || attempt >= self.config.max_retries {
            self.state = State::Idle;
            return vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason,
                },
            )];
        }

        let retry_at = now.saturating_add(self.backoff_ms(attempt + 1) / 1000);
        self.state = State::Failed {
            retry_at,
            attempt: attempt + 1,
        };
        Vec::new()
    }

    fn backoff_ms(&self, attempt: u32) -> u64 {
        let base = self.config.base_backoff_ms;
        let max = self.config.max_backoff_ms;
        let exp = base.saturating_mul(2_u64.saturating_pow(attempt.min(16)));
        let delay = exp.min(max);
        let jitter = self.jitter(attempt);
        delay.saturating_add(jitter)
    }

    fn jitter(&self, attempt: u32) -> u64 {
        if self.config.jitter_ms == 0 {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        self.platform_id().hash(&mut hasher);
        attempt.hash(&mut hasher);
        let hash = hasher.finish();
        hash % self.config.jitter_ms
    }

    fn next_call_id(&mut self, now: u64) -> String {
        self.request_counter += 1;
        format!("{}-{now}-{}", self.platform_id(), self.request_counter)
    }

    fn next_local_tag(&mut self, now: u64) -> String {
        self.request_counter += 1;
        format!("{}-{now}-{}", self.platform_id(), self.request_counter)
    }

    fn next_branch(&mut self, call_id: &str, cseq: u32) -> String {
        self.request_counter += 1;
        format!("z9hG4bK-{}-{cseq}-{}", call_id, self.request_counter)
    }

    fn platform_id(&self) -> &str {
        self.config
            .local_uri
            .user()
            .unwrap_or(self.config.local_uri.host())
    }
}

fn extract_challenge(msg: &SipMessage) -> Result<Option<DigestChallenge>, CascadeError> {
    let Some(value) = msg.headers().get(&HeaderName::WwwAuthenticate) else {
        return Ok(None);
    };
    match DigestChallenge::parse(value.as_str()) {
        Ok(c) => Ok(Some(c)),
        Err(e) => Err(CascadeError::MalformedSip(format!(
            "failed to parse WWW-Authenticate: {e}"
        ))),
    }
}

fn parse_expires(msg: &SipMessage, default: u32) -> u32 {
    msg.headers()
        .get(&HeaderName::Expires)
        .and_then(|v| v.as_str().trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests;
