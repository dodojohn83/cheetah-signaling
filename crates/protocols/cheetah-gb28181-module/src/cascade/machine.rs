//! `Gb28181Cascade` state machine implementation.

use std::sync::{Arc, Mutex};

use cheetah_signal_types::hash::stable_hash_u64;
use secrecy::ExposeSecret;

use cheetah_gb28181_core::{
    DigestChallenge, DigestClient, DigestContext, DigestReplayCache, DigestResponse, HeaderName,
    SipMessage,
};

use super::keepalive::build_keepalive_message;
use super::registration::build_register_request;
use super::{
    AuthorizationContext, CascadeConfig, CascadeCredentialProvider, CascadeError, CascadeEvent,
    CascadeInput, CascadeOutput, CatalogProvider, Gb28181Cascade, InboundAuthContext, Registered,
    Registering, State,
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
            report_counter: 0,
            auth: None,
            catalog_provider: None,
            inbound_auth,
            bridges: std::collections::BTreeMap::new(),
            report_queue: std::collections::VecDeque::new(),
            report_state: std::collections::HashMap::new(),
            report_state_order: std::collections::VecDeque::new(),
            subscriptions: std::collections::BTreeMap::new(),
        })
    }

    /// Attaches a catalog provider for handling upstream `Catalog` queries.
    pub fn with_catalog_provider(mut self, catalog_provider: Arc<dyn CatalogProvider>) -> Self {
        self.catalog_provider = Some(catalog_provider);
        self
    }

    /// Processes a single input and returns ordered outputs.
    ///
    /// Errors are non-fatal: the state machine is left unchanged and the caller
    /// (typically a driver) should log the failure and continue. Retry and
    /// timeout paths driven by `CascadeEvent::Tick` will eventually drive the
    /// cascade forward.
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
            CascadeEvent::Report { event } => super::report::enqueue(self, input.now, *event),
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
                let auth = match prev.challenge.as_ref() {
                    Some(challenge) => {
                        match self.build_authorization(challenge, reg.cseq, reg.attempt, now) {
                            Ok(auth) => Some(auth),
                            Err(e) => {
                                return Ok(self.fail_or_retry(
                                    now,
                                    reg.attempt,
                                    false,
                                    format!("authorization build failed for refresh: {e}"),
                                ));
                            }
                        }
                    }
                    None => None,
                };
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
                let auth = match prev.challenge.as_ref() {
                    Some(challenge) => {
                        match self.build_authorization(challenge, reg.cseq, reg.attempt, now) {
                            Ok(auth) => Some(auth),
                            Err(e) => {
                                return Ok(self.fail_or_retry(
                                    now,
                                    reg.attempt,
                                    true,
                                    format!("authorization build failed for deregister: {e}"),
                                ));
                            }
                        }
                    }
                    None => None,
                };
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
        let mut outputs = super::bridge::on_tick(self, now)?;

        let subscription_outputs = super::subscription::on_tick(self, now);
        outputs.extend(subscription_outputs);

        let more = match self.state.clone() {
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
        }?;

        outputs.extend(more);
        let report_outputs = super::report::flush(self, now)?;
        outputs.extend(report_outputs);
        Ok(outputs)
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
                self.subscriptions.clear();
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
            Method::Subscribe => super::subscription::handle_subscribe(self, now, msg),
            _ => super::request_handler::handle_request(self, now, msg),
        }
    }

    pub(super) fn build_authorization(
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

    pub(super) fn fail_or_retry(
        &mut self,
        now: u64,
        attempt: u32,
        is_deregister: bool,
        reason: String,
    ) -> Vec<CascadeOutput> {
        if is_deregister || attempt >= self.config.max_retries {
            self.subscriptions.clear();
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
        self.subscriptions.clear();
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
        let hash = stable_hash_u64(&(self.platform_id(), attempt));
        hash % self.config.jitter_ms
    }

    pub(super) fn next_call_id(&mut self, now: u64) -> String {
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

pub(super) fn extract_challenge(msg: &SipMessage) -> Result<Option<DigestChallenge>, CascadeError> {
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

pub(super) fn parse_expires(msg: &SipMessage, default: u32) -> Result<u32, CascadeError> {
    let Some(value) = msg.headers().get(&HeaderName::Expires) else {
        return Ok(default);
    };
    let trimmed = value.as_str().trim();
    if trimmed.is_empty() {
        return Err(CascadeError::MalformedSip(
            "empty Expires header".to_string(),
        ));
    }
    trimmed
        .parse::<u32>()
        .map_err(|_| CascadeError::MalformedSip(format!("non-numeric Expires header: {trimmed}")))
}
