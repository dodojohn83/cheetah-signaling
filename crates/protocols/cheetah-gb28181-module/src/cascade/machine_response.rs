//! Response handling for the `Gb28181Cascade` state machine.
//!
//! This module holds the SIP response dispatch and the REGISTER, deregister,
//! and keepalive response handlers, including the digest challenge/resend path.
//! It is a sibling of [`super::machine`], which owns the command-driven
//! registration flow and shared helpers.

use cheetah_gb28181_core::{Method, SipMessage};

use super::machine::{extract_challenge, parse_expires};
use super::registration::build_register_request;
use super::{
    CascadeCredentialProvider, CascadeError, CascadeOutput, Gb28181Cascade, Keepalive, Registered,
    Registering, State,
};
use crate::events::Gb28181Event;

impl<P: CascadeCredentialProvider> Gb28181Cascade<P> {
    pub(super) fn on_response(
        &mut self,
        now: u64,
        msg: SipMessage,
    ) -> Result<Vec<CascadeOutput>, CascadeError> {
        let (cseq_num, cseq_method, call_id) = match &msg {
            SipMessage::Response { .. } => {
                let cseq = msg
                    .cseq()
                    .map_err(|e| CascadeError::MalformedSip(e.to_string()))?;
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
            Method::Notify => Ok(super::subscription::handle_response(self, now, msg)),
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

        if status.code >= 300 {
            let reason = format!("REGISTER redirected with {} {}", status.code, status.reason);
            return Ok(self.fail_or_retry(now, reg.attempt, false, reason));
        }

        if (200..300).contains(&status.code) {
            let expires = parse_expires(&msg, self.config.register_interval_seconds)?;
            if expires == 0 {
                // The upstream removed the binding; do not schedule a refresh.
                self.subscriptions.clear();
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
            self.subscriptions.clear();
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
            self.subscriptions.clear();
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
            self.subscriptions.clear();
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
            self.subscriptions.clear();
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
            self.subscriptions.clear();
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
}
