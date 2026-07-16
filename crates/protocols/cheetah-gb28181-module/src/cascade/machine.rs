//! `Gb28181Cascade` state machine implementation.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use secrecy::ExposeSecret;

use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, DigestContext, DigestReplayCache, DigestResponse, HeaderName,
    Method, SipMessage,
};

use super::keepalive::build_keepalive_message;
use super::registration::build_register_request;
use super::{
    AuthorizationContext, CascadeConfig, CascadeCredentialProvider, CascadeError, CascadeEvent,
    CascadeInput, CascadeOutput, CatalogProvider, Gb28181Cascade, InboundAuthContext, Keepalive,
    Registered, Registering, State,
};
use crate::events::Gb28181Event;

impl<P: CascadeCredentialProvider> Gb28181Cascade<P> {
    /// Creates a new cascade state machine.
    pub fn new(config: CascadeConfig, provider: P) -> Result<Self, CascadeError> {
        let inbound_auth = if let Some(secret) = &config.catalog_inbound_digest_server_secret {
            let digest = DigestContext::new(config.realm.clone(), secret.expose_secret())?;
            Some(Arc::new(InboundAuthContext {
                digest,
                replay: Mutex::new(DigestReplayCache::new(1024)),
            }))
        } else {
            None
        };
        Ok(Self {
            config,
            provider,
            state: State::Idle,
            request_counter: 0,
            bridge_counter: 0,
            auth: None,
            catalog_provider: None,
            inbound_auth,
            bridges: std::collections::BTreeMap::new(),
        })
    }

    /// Attaches a catalog provider for handling upstream `Catalog` queries.
    pub fn with_catalog_provider(mut self, catalog_provider: Arc<dyn CatalogProvider>) -> Self {
        self.catalog_provider = Some(catalog_provider);
        self
    }

    /// Processes a single input and returns ordered outputs.
    pub fn process(&mut self, input: CascadeInput) -> Result<Vec<CascadeOutput>, CascadeError> {
        match input.event {
            CascadeEvent::Register => self.on_register(input.now),
            CascadeEvent::Deregister => self.on_deregister(input.now),
            CascadeEvent::Request(msg) => Ok(self.on_request(input.now, *msg)),
            CascadeEvent::Response(msg) => self.on_response(input.now, *msg),
            CascadeEvent::BridgeMediaReady {
                bridge_id,
                answer_sdp,
            } => super::bridge::on_media_ready(self, input.now, bridge_id, answer_sdp),
            CascadeEvent::BridgeMediaStop { bridge_id } => {
                super::bridge::on_media_stop(self, input.now, bridge_id)
            }
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
        match self.state.clone() {
            State::Registered(reg) if now >= reg.refresh_at => {
                // Trigger a refresh.
                self.on_register(now)
            }
            State::Registered(reg) => self.on_keepalive_tick(now, reg),
            State::Failed { retry_at, .. } if now >= retry_at => {
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

    fn on_keepalive_tick(
        &mut self,
        now: u64,
        mut reg: Registered,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        if let Some(pending_until) = reg.keepalive.pending_until {
            if now < pending_until {
                return Ok(Vec::new());
            }
            reg.keepalive.failures += 1;
            reg.keepalive.pending_until = None;
            if reg.keepalive.failures >= self.config.keepalive_max_failures {
                self.state = State::Idle;
                return Ok(vec![CascadeOutput::EmitEvent(
                    Gb28181Event::CascadePlatformDisconnected {
                        domain_id: self.config.domain_id.clone(),
                        platform_id: self.platform_id().to_string(),
                        reason: "keepalive timed out".to_string(),
                    },
                )]);
            }
            self.state = State::Registered(reg);
            return Ok(Vec::new());
        }

        if now < reg.keepalive.next_at {
            return Ok(Vec::new());
        }

        reg.keepalive.sn = reg
            .keepalive
            .sn
            .checked_add(1)
            .ok_or_else(|| CascadeError::Internal("keepalive SN overflow".to_string()))?;
        reg.keepalive.cseq = reg
            .keepalive
            .cseq
            .checked_add(1)
            .ok_or_else(|| CascadeError::Internal("keepalive CSeq overflow".to_string()))?;
        let branch = self.next_branch(&reg.keepalive.call_id, reg.keepalive.cseq);
        let msg = build_keepalive_message(
            &self.config,
            &reg.keepalive.call_id,
            reg.keepalive.cseq,
            &reg.local_tag,
            &branch,
            reg.keepalive.sn,
            self.platform_id(),
        )?;
        reg.keepalive.pending_until =
            Some(now.saturating_add(self.config.keepalive_timeout_seconds.into()));
        reg.keepalive.next_at = now.saturating_add(self.config.keepalive_interval_seconds.into());

        self.state = State::Registered(reg);
        Ok(vec![CascadeOutput::SendRequest(msg)])
    }

    fn on_response(
        &mut self,
        now: u64,
        msg: SipMessage,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let (cseq_num, cseq_method, call_id) = match &msg {
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

        match cseq_method {
            Method::Register => match &self.state {
                State::Registering(reg) if reg.call_id == call_id => {
                    let reg = reg.clone();
                    self.handle_register_response(now, msg, reg)
                }
                State::Deregistering(reg) if reg.call_id == call_id => {
                    let reg = reg.clone();
                    self.handle_deregister_response(now, msg, reg)
                }
                _ => Ok(Vec::new()),
            },
            Method::Message => match &self.state {
                State::Registered(reg)
                    if reg.keepalive.call_id == call_id && reg.keepalive.cseq == cseq_num =>
                {
                    let reg = reg.clone();
                    Ok(self.handle_keepalive_response(now, msg, reg))
                }
                _ => Ok(Vec::new()),
            },
            _ => Ok(Vec::new()),
        }
    }

    fn on_request(&mut self, now: u64, msg: SipMessage) -> Vec<CascadeOutput> {
        use cheetah_gb28181_core::Method;
        let method = match &msg {
            SipMessage::Request { line, .. } => line.method.clone(),
            _ => return Vec::new(),
        };
        match method {
            Method::Invite | Method::Ack | Method::Bye | Method::Cancel => {
                super::bridge::handle_request(self, now, msg)
            }
            _ => super::request_handler::handle_request(self, now, msg),
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

        if status.code >= 300 {
            let reason = format!("REGISTER redirected with {} {}", status.code, status.reason);
            return Ok(self.fail_or_retry(now, reg.attempt, false, reason));
        }

        if (200..300).contains(&status.code) {
            let expires = parse_expires(&msg, self.config.register_interval_seconds);
            if expires == 0 {
                // The upstream removed the binding; do not schedule a refresh.
                self.state = State::Idle;
                return Ok(vec![CascadeOutput::EmitEvent(
                    Gb28181Event::CascadePlatformDisconnected {
                        domain_id: self.config.domain_id.clone(),
                        platform_id: self.platform_id().to_string(),
                        reason: "upstream granted zero expiry".to_string(),
                    },
                )]);
            }

            // A 200 OK may not repeat WWW-Authenticate; carry the challenge
            // from the 401 that authenticated this transaction forward.
            let challenge = extract_challenge(&msg)
                .ok()
                .flatten()
                .or_else(|| reg.challenge.clone());

            let expires_u64 = u64::from(expires);
            let margin = u64::from(self.config.register_refresh_margin_seconds)
                .min(expires_u64 / 2)
                .max(1);
            let refresh_after = expires_u64.saturating_sub(margin).max(1);
            let refresh_at = now.saturating_add(refresh_after);

            let keepalive_call_id = self.next_call_id(now);
            let keepalive = Keepalive::new(
                now.saturating_add(self.config.keepalive_interval_seconds.into()),
                keepalive_call_id,
                0,
            );

            let registered = Registered {
                cseq: reg.cseq,
                call_id: reg.call_id.clone(),
                local_tag: reg.local_tag.clone(),
                refresh_at,
                challenge,
                keepalive,
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

        if status.code >= 300 {
            self.state = State::Idle;
            return Ok(vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason: format!(
                        "deregister redirected with {} {}",
                        status.code, status.reason
                    ),
                },
            )]);
        }

        if (200..300).contains(&status.code) {
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

    fn handle_keepalive_response(
        &mut self,
        now: u64,
        msg: SipMessage,
        mut reg: Registered,
    ) -> Vec<CascadeOutput> {
        let (status, body) = match msg {
            SipMessage::Response { line, body, .. } => (line, body),
            _ => unreachable!("caller ensures a response"),
        };

        // Any non-2xx final response (including 3xx redirects) is a
        // transport failure. The business outcome of a 200 OK is further
        // checked by parsing the XML body. Only clear the pending timeout for
        // final responses; provisional responses must retain it so a lost final
        // response still triggers the timeout.
        if status.code >= 300 {
            reg.keepalive.pending_until = None;
            return self.keepalive_failure(now, reg, status.code, &status.reason);
        }

        if (200..300).contains(&status.code) {
            reg.keepalive.pending_until = None;
            let business_ok = if body.is_empty() {
                // Empty body is treated as transport-level success.
                true
            } else {
                matches!(
                    crate::xml::parse_keepalive_response(&body).map(|r| r.result),
                    Ok(ref r) if r.eq_ignore_ascii_case("OK")
                )
            };

            if business_ok {
                reg.keepalive.failures = 0;
                reg.keepalive.next_at =
                    now.saturating_add(self.config.keepalive_interval_seconds.into());
                self.state = State::Registered(reg);
                return Vec::new();
            }

            return self.keepalive_failure(now, reg, status.code, "upstream rejected keepalive");
        }

        // Provisional response; wait for final.
        self.state = State::Registered(reg);
        Vec::new()
    }

    fn keepalive_failure(
        &mut self,
        now: u64,
        mut reg: Registered,
        code: u16,
        reason: &str,
    ) -> Vec<CascadeOutput> {
        reg.keepalive.failures += 1;
        if reg.keepalive.failures >= self.config.keepalive_max_failures {
            self.state = State::Idle;
            return vec![CascadeOutput::EmitEvent(
                Gb28181Event::CascadePlatformDisconnected {
                    domain_id: self.config.domain_id.clone(),
                    platform_id: self.platform_id().to_string(),
                    reason: format!("keepalive failed with {code} {reason}"),
                },
            )];
        }
        reg.keepalive.next_at = now.saturating_add(self.config.keepalive_interval_seconds.into());
        self.state = State::Registered(reg);
        Vec::new()
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

        // Store the challenge used for the next refresh/deregister. The
        // `Registering` field holds it even when `previous` is None, so an
        // initial registration caches the challenge after its first 401.
        if !reg.is_deregister {
            reg.challenge = Some(challenge.clone());
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
        &mut self,
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

        let needs_new = self
            .auth
            .as_ref()
            .is_none_or(|auth| auth.challenge != *challenge);
        if needs_new {
            let client = DigestClient::new()
                .allow_md5(self.config.allow_md5)
                .qop(challenge.qop)?;
            self.auth = Some(AuthorizationContext {
                challenge: challenge.clone(),
                client,
            });
        }

        let platform_id = self.platform_id().to_string();
        let password = self
            .provider
            .password_for(&self.config.credential_ref)
            .ok_or(CascadeError::NoCredentials)?;
        let auth = self.auth.as_mut().ok_or_else(|| {
            CascadeError::Internal("digest auth context missing after creation".to_string())
        })?;
        let cnonce = DigestClient::derive_cnonce(
            &password,
            &format!("{platform_id}-{cseq}-{attempt}-{now}"),
        )?;
        let username = self.config.local_uri.user().unwrap_or("");
        let uri = self.config.upstream.encode();
        Ok(auth
            .client
            .authorize(username, &password, "REGISTER", &uri, challenge, &cnonce)?)
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

        let challenge = previous.as_ref().and_then(|p| p.challenge.clone());
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
            challenge,
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

    pub(super) fn next_local_tag(&mut self, now: u64) -> String {
        self.request_counter += 1;
        format!("{}-{now}-{}", self.platform_id(), self.request_counter)
    }

    pub(super) fn next_cseq(&mut self) -> u32 {
        self.request_counter += 1;
        self.request_counter as u32
    }

    pub(super) fn next_branch(&mut self, call_id: &str, cseq: u32) -> String {
        self.request_counter += 1;
        format!("z9hG4bK-{}-{cseq}-{}", call_id, self.request_counter)
    }

    pub(super) fn platform_id(&self) -> &str {
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
