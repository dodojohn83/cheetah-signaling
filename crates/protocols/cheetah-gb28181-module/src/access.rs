//! Sans-I/O GB28181 access state machine.
//!
//! Implements [`cheetah_gb28181_core::GbAccessMachine`] for the GB28181 module.
//! The input/output contract lives in `cheetah-gb28181-core` so the driver can
//! execute the machine without depending on this module.

use crate::config::{AuthPolicy, Gb28181DomainConfig};
use crate::error::AccessError;
use crate::events::{DevicePresence, Gb28181Event};
use crate::ports::CredentialProvider;
use crate::registration::RegistrationTable;
use crate::types::DeviceId;
use crate::xml::{
    XmlLimits, extract_alarm, extract_catalog, extract_device_control_response,
    extract_device_info, extract_device_status, extract_keepalive, extract_mobile_position,
    extract_record_info, parse_xml,
};
use cheetah_gb28181_core::{
    AccessInput, AccessOutput, AuthRateLimiter, DigestContext, DigestQop, DigestReplayCache,
    EndpointRoute, GbAccessMachine, HeaderName, Method, SipMessage,
};
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::warn;

mod parse;
mod response;

use parse::{
    device_id_from_request, parse_authorization, parse_contact_header, parse_expires_header,
    resolve_expires,
};
use response::{
    build_challenge_response, build_error_response, build_message_response,
    build_rate_limited_response, build_success_response,
};

/// Sans-I/O state machine for GB28181 device access.
pub struct Gb28181Access<P: CredentialProvider> {
    config: Gb28181DomainConfig,
    digest_context: DigestContext,
    replay_cache: DigestReplayCache,
    auth_rate_limiter: AuthRateLimiter,
    credential_provider: P,
    tag_counter: AtomicU64,
    registrations: RegistrationTable,
}

impl<P: CredentialProvider> std::fmt::Debug for Gb28181Access<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181Access")
            .field("config", &self.config)
            .field("digest_context", &self.digest_context)
            .field("replay_cache", &self.replay_cache)
            .field("auth_rate_limiter", &self.auth_rate_limiter)
            .field("credential_provider", &"<dyn CredentialProvider>")
            .field("tag_counter", &self.tag_counter)
            .finish()
    }
}

impl<P: CredentialProvider> Gb28181Access<P> {
    /// Creates a new access handler for the supplied domain config.
    ///
    /// Returns an error if the digest secret is too short (less than 32 bytes).
    pub fn new(config: Gb28181DomainConfig, credential_provider: P) -> Result<Self, AccessError> {
        let max_registrations = config.max_registrations();
        let ctx = DigestContext::new(config.realm(), config.digest_secret().expose_secret())
            .map_err(|e| AccessError::Internal(e.to_string()))?
            .allow_md5(config.allow_md5())
            .preferred_algorithm(config.preferred_algorithm())
            .qop(Some(DigestQop::Auth))
            .map_err(|e| AccessError::Internal(e.to_string()))?;
        let auth_rate_limiter = AuthRateLimiter::new(
            config.auth_max_failures_per_source(),
            config.auth_rate_window_seconds(),
            config.auth_rate_max_sources(),
        );
        Ok(Self {
            config,
            digest_context: ctx,
            replay_cache: DigestReplayCache::new(1024),
            auth_rate_limiter,
            credential_provider,
            tag_counter: AtomicU64::new(1),
            registrations: RegistrationTable::new(max_registrations),
        })
    }
}

impl<P: CredentialProvider> GbAccessMachine for Gb28181Access<P> {
    type Event = Gb28181Event;
    type Error = AccessError;

    /// Processes a single SIP message and returns the ordered outputs.
    fn process(
        &mut self,
        input: AccessInput,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        match &input.message {
            SipMessage::Request { line, .. } if line.method == Method::Register => {
                self.process_register(input)
            }
            SipMessage::Request { line, .. } if line.method == Method::Message => {
                self.process_message(input)
            }
            SipMessage::Request { .. } => Ok(vec![AccessOutput::SendResponse(
                build_error_response(&input.message, 501, "Not Implemented", self.next_tag()),
            )]),
            SipMessage::Response { .. } => Ok(Vec::new()),
        }
    }

    /// Advances the registration timer wheel and returns any resulting events.
    fn tick(&mut self, now: u64) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        let heartbeat_timeout = self.config.heartbeat_timeout_seconds();
        let mut outputs = Vec::new();
        let mut expired = Vec::new();

        for (device_id, reg) in self.registrations.iter_mut() {
            if now.saturating_sub(reg.registered_at) >= reg.expires as u64 {
                expired.push(device_id.clone());
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::DeviceUnregistered {
                    domain_id: self.config.domain_id().clone(),
                    device_id: device_id.clone(),
                    source: reg.source(),
                }));
                continue;
            }

            if !reg.offline && now.saturating_sub(reg.last_seen) >= heartbeat_timeout {
                reg.offline = true;
                outputs.push(AccessOutput::EmitEvent(
                    Gb28181Event::DevicePresenceChanged {
                        domain_id: self.config.domain_id().clone(),
                        device_id: device_id.clone(),
                        source: reg.source(),
                        presence: DevicePresence::Offline,
                    },
                ));
            }
        }

        for device_id in expired {
            self.registrations.remove(&device_id);
        }

        Ok(outputs)
    }
}

impl<P: CredentialProvider> Gb28181Access<P> {
    fn process_register(
        &mut self,
        input: AccessInput,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        let AccessInput {
            source,
            now,
            message,
        } = input;
        let SipMessage::Request { line, headers, .. } = &message else {
            return Err(AccessError::Internal("expected request".to_string()));
        };

        let device_id = match device_id_from_request(line, headers) {
            Ok(id) => id,
            Err(AccessError::InvalidDeviceId) => {
                return Ok(self.bad_request_response(&message));
            }
            Err(e) => return Err(e),
        };
        let (contact_uri, contact_expires) = match parse_contact_header(headers) {
            Ok(v) => v,
            Err(AccessError::InvalidContact | AccessError::InvalidExpires) => {
                return Ok(self.bad_request_response(&message));
            }
            Err(e) => return Err(e),
        };
        let expires_header = match parse_expires_header(headers) {
            Ok(v) => v,
            Err(AccessError::InvalidExpires) => {
                return Ok(self.bad_request_response(&message));
            }
            Err(e) => return Err(e),
        };
        let expires = resolve_expires(contact_expires, expires_header, &self.config);

        let user_agent = headers
            .get(&HeaderName::UserAgent)
            .map(|v| v.as_str().to_string());

        // In ChallengeOptional mode we accept devices that do not present
        // credentials. If credentials are present and the device is known,
        // validate them; invalid credentials must be rejected even in optional
        // mode.
        let mut auth_ok = false;
        let mut auth_attempted = false;
        if let Some(auth_header) = headers.get(&HeaderName::Authorization) {
            // Rate limiting is applied before any digest computation so that a
            // brute-force source cannot force expensive hashing. A blocked
            // source is rejected with 429 and a Retry-After hint.
            if self.auth_rate_limiter.is_blocked(source.ip(), now) {
                return Ok(self.rate_limited_response(&message, now, source.ip()));
            }
            if self.config.auth_policy() == AuthPolicy::Required {
                let digest = match parse_authorization(auth_header.as_str()) {
                    Ok(d) => d,
                    Err(
                        cheetah_gb28181_core::DigestError::Malformed(_)
                        | cheetah_gb28181_core::DigestError::UnknownAlgorithm
                        | cheetah_gb28181_core::DigestError::InvalidQop,
                    ) => {
                        return Ok(self.bad_request_response(&message));
                    }
                    Err(_) => return self.auth_failed(&message, now, false, source),
                };
                let password = match self.credential_provider.password_for(&device_id) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return self.auth_failed(&message, now, false, source);
                    }
                    Err(e) => {
                        warn!(device_id = %device_id, error = %e, "credential provider backend error during REGISTER");
                        return self.authentication_failure_response(&message, now, false);
                    }
                };
                let request_uri = line.uri.encode();
                match self.digest_context.validate(
                    &digest,
                    &Method::Register,
                    &request_uri,
                    &password,
                    &mut self.replay_cache,
                    now,
                ) {
                    Ok(()) => auth_ok = true,
                    Err(cheetah_gb28181_core::DigestError::StaleNonce) => {
                        return self.authentication_failure_response(&message, now, true);
                    }
                    Err(_) => return self.auth_failed(&message, now, false, source),
                }
            } else {
                // ChallengeOptional: missing password is acceptable, but a backend
                // error must not be treated as "no password" and fall through to an
                // unauthenticated acceptance. When a password is configured and the
                // device presents an Authorization header, the digest must validate.
                match self.credential_provider.password_for(&device_id) {
                    Ok(Some(password)) => {
                        let digest = match parse_authorization(auth_header.as_str()) {
                            Ok(d) => d,
                            Err(
                                cheetah_gb28181_core::DigestError::Malformed(_)
                                | cheetah_gb28181_core::DigestError::UnknownAlgorithm
                                | cheetah_gb28181_core::DigestError::InvalidQop,
                            ) => return Ok(self.bad_request_response(&message)),
                            Err(_) => {
                                return self.auth_failed(&message, now, false, source);
                            }
                        };
                        let request_uri = line.uri.encode();
                        match self.digest_context.validate(
                            &digest,
                            &Method::Register,
                            &request_uri,
                            &password,
                            &mut self.replay_cache,
                            now,
                        ) {
                            Ok(()) => auth_ok = true,
                            Err(cheetah_gb28181_core::DigestError::StaleNonce) => {
                                return self.authentication_failure_response(&message, now, true);
                            }
                            Err(_) => {
                                self.auth_rate_limiter.record_failure(source.ip(), now);
                                auth_attempted = true;
                            }
                        }
                    }
                    Ok(None) => {
                        // No password configured; ignore the header and accept as
                        // unauthenticated in ChallengeOptional mode.
                    }
                    Err(e) => {
                        warn!(device_id = %device_id, error = %e, "credential provider backend error during REGISTER");
                        return self.authentication_failure_response(&message, now, false);
                    }
                }
            }
        }

        if auth_ok {
            // A successful authentication clears any accumulated failures so a
            // legitimate device is never penalised by earlier bad attempts.
            self.auth_rate_limiter.record_success(source.ip());
        }

        if auth_ok
            || (self.config.auth_policy() == AuthPolicy::ChallengeOptional && !auth_attempted)
        {
            self.register_accepted(
                &message,
                &contact_uri,
                expires,
                device_id,
                source,
                user_agent,
                now,
            )
        } else {
            self.authentication_failure_response(&message, now, false)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn register_accepted(
        &mut self,
        message: &SipMessage,
        contact_uri: &cheetah_gb28181_core::SipUri,
        expires: u32,
        device_id: DeviceId,
        source: std::net::SocketAddr,
        user_agent: Option<String>,
        now: u64,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        if expires == 0 {
            self.registrations.remove(&device_id);
            let response = build_success_response(message, contact_uri, expires, self.next_tag());
            Ok(vec![
                AccessOutput::SendResponse(response),
                AccessOutput::EmitEvent(Gb28181Event::DeviceUnregistered {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                }),
            ])
        } else {
            let contact = contact_uri.encode();
            // Build the endpoint route from the authenticated REGISTER: the
            // observed source, the top Via `received`/`rport`, and the Contact
            // URI. This is the only sanctioned way to (re)establish the send
            // route; keepalive/MESSAGE packets never move it.
            let top_via = message
                .headers()
                .get_all(&HeaderName::Via)
                .next()
                .map(|v| v.as_str());
            let route =
                EndpointRoute::from_registration(source, top_via, Some(contact_uri.clone()));
            let registration = match self.registrations.upsert(
                device_id.clone(),
                route,
                contact.clone(),
                expires,
                now,
                user_agent.clone(),
            ) {
                Ok(registration) => registration,
                Err(AccessError::RegistrationTableFull) => {
                    return Ok(vec![AccessOutput::SendResponse(build_error_response(
                        message,
                        503,
                        "Service Unavailable",
                        self.next_tag(),
                    ))]);
                }
                Err(e) => return Err(e),
            };
            let response = build_success_response(message, contact_uri, expires, self.next_tag());
            Ok(vec![
                AccessOutput::SendResponse(response),
                AccessOutput::EmitEvent(Gb28181Event::DeviceRegistered {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    contact,
                    expires,
                    user_agent,
                    registration_sequence: registration.registration_sequence,
                }),
            ])
        }
    }

    fn process_message(
        &mut self,
        input: AccessInput,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        let AccessInput {
            source,
            now,
            message,
        } = input;
        let request = match &message {
            SipMessage::Request { .. } => &message,
            _ => return Err(AccessError::Internal("expected request".to_string())),
        };

        match self.process_message_body(source, now, request) {
            Ok(outputs) => Ok(outputs),
            Err(
                AccessError::InvalidDeviceId
                | AccessError::InvalidXml(_)
                | AccessError::UnsupportedCmdType(_),
            ) => Ok(vec![AccessOutput::SendResponse(build_error_response(
                request,
                400,
                "Bad Request",
                self.next_tag(),
            ))]),
            Err(AccessError::NotRegistered) => Ok(vec![AccessOutput::SendResponse(
                build_error_response(request, 403, "Forbidden", self.next_tag()),
            )]),
            Err(e) => Err(e),
        }
    }

    fn process_message_body(
        &mut self,
        source: std::net::SocketAddr,
        now: u64,
        message: &SipMessage,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        let SipMessage::Request { headers, body, .. } = message else {
            return Err(AccessError::Internal("expected request".to_string()));
        };

        let from = headers
            .get(&HeaderName::From)
            .map(|v| v.as_str())
            .ok_or(AccessError::InvalidDeviceId)?;
        let from_device_id =
            parse::device_from_address(from).ok_or(AccessError::InvalidDeviceId)?;

        let root = parse_xml(body, &XmlLimits::default())?;
        let cmd_type = root
            .child_text("CmdType")
            .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
        let xml_device_id = root
            .require_child_text("DeviceID")
            .map_err(|_| AccessError::InvalidXml("missing DeviceID".to_string()))?;

        if from_device_id.as_ref() != xml_device_id {
            return Err(AccessError::InvalidDeviceId);
        }
        let device_id = DeviceId::new(&xml_device_id).ok_or(AccessError::InvalidDeviceId)?;

        let domain_id = self.config.domain_id().clone();

        // Parse the command payload before touching the registration table.
        // A malformed or unknown command must not commit an online presence
        // transition whose event would then be discarded when we return a 400
        // response. Re-use the already parsed XML tree to avoid double parsing.
        let event = match cmd_type.as_str() {
            "Keepalive" => {
                let keepalive = extract_keepalive(&root)?;
                Gb28181Event::Keepalive {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    status: keepalive.status,
                }
            }
            "Catalog" => {
                let catalog = extract_catalog(&root)?;
                Gb28181Event::CatalogReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: catalog.sn,
                    sum_num: catalog.sum_num,
                    num: catalog.num,
                    items: catalog.items,
                }
            }
            "DeviceInfo" => {
                let info = extract_device_info(&root)?;
                Gb28181Event::DeviceInfoReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: info.sn,
                    result: info.result,
                    manufacturer: info.manufacturer,
                    model: info.model,
                    firmware: info.firmware,
                }
            }
            "DeviceStatus" => {
                let status = extract_device_status(&root)?;
                Gb28181Event::DeviceStatusReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: status.sn,
                    result: status.result,
                    online: status.online,
                    status: status.status,
                    reason: status.reason,
                    invalid_equip: status.invalid_equip,
                }
            }
            "Alarm" => {
                let alarm = extract_alarm(&root)?;
                Gb28181Event::AlarmReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: alarm.sn,
                    priority: alarm.priority,
                    method: alarm.method,
                    alarm_type: alarm.alarm_type,
                    time: alarm.time,
                    info: alarm.info,
                }
            }
            "MobilePosition" => {
                let pos = extract_mobile_position(&root)?;
                Gb28181Event::MobilePositionReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: pos.sn,
                    time: pos.time,
                    longitude: pos.longitude,
                    latitude: pos.latitude,
                    speed: pos.speed,
                    direction: pos.direction,
                    altitude: pos.altitude,
                }
            }
            "RecordInfo" => {
                let info = extract_record_info(&root)?;
                Gb28181Event::RecordInfoReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: info.sn,
                    name: info.name,
                    sum_num: info.sum_num,
                    num: info.num,
                    items: info.items,
                }
            }
            "DeviceControl" => {
                let resp = extract_device_control_response(&root)?;
                Gb28181Event::DeviceControlResponseReceived {
                    domain_id: domain_id.clone(),
                    device_id: device_id.clone(),
                    source,
                    sn: resp.sn,
                    result: resp.result,
                }
            }
            other => return Err(AccessError::UnsupportedCmdType(other.to_string())),
        };

        let Some(touch) = self.registrations.touch(&device_id, source, now) else {
            // Business messages from a device that is not currently registered
            // must not bypass the registration policy.
            return Err(AccessError::NotRegistered);
        };

        if touch.source_drift {
            // A keepalive/MESSAGE arrived from an address other than the one
            // established by the authenticated REGISTER. The stored send route
            // is intentionally left unchanged (source hijack regression); the
            // packet is still accepted for presence to avoid dropping a
            // legitimate device that roamed without re-registering. Only an
            // authenticated REGISTER may move the send route.
            warn!(
                device_id = %device_id,
                "gb28181 keepalive/MESSAGE source differs from registered route; endpoint not moved"
            );
        }

        let mut outputs = Vec::with_capacity(3);
        if touch.was_offline {
            // Presence events report the authoritative registered route, not the
            // (spoofable) packet source, so online/offline/expiry events stay
            // consistent and a drifting keepalive cannot publish an untrusted
            // address.
            let presence_source = self.registrations.send_target(&device_id).unwrap_or(source);
            outputs.push(AccessOutput::EmitEvent(
                Gb28181Event::DevicePresenceChanged {
                    domain_id,
                    device_id: device_id.clone(),
                    source: presence_source,
                    presence: DevicePresence::Online,
                },
            ));
        }

        outputs.push(AccessOutput::EmitEvent(event));
        outputs.push(AccessOutput::SendResponse(build_message_response(
            message,
            self.next_tag(),
        )));
        Ok(outputs)
    }

    /// Returns the resolved send target for a registered device.
    ///
    /// This is the address the server should send server-initiated requests
    /// (and out-of-dialog responses) to, computed from the NAT/`rport` policy
    /// of the endpoint route established at registration. Returns `None` when
    /// the device is not currently registered.
    pub fn device_send_target(&self, device_id: &DeviceId) -> Option<SocketAddr> {
        self.registrations.send_target(device_id)
    }

    /// Returns the full endpoint route for a registered device, if any.
    pub fn device_route(&self, device_id: &DeviceId) -> Option<EndpointRoute> {
        self.registrations.route(device_id)
    }

    fn next_tag(&self) -> String {
        let n = self.tag_counter.fetch_add(1, Ordering::Relaxed);
        format!("gb{n}")
    }

    fn bad_request_response(&self, request: &SipMessage) -> Vec<AccessOutput<Gb28181Event>> {
        vec![AccessOutput::SendResponse(build_error_response(
            request,
            400,
            "Bad Request",
            self.next_tag(),
        ))]
    }

    /// Records a per-source authentication failure and returns the challenge
    /// response. Used when a device presents credentials that fail to validate,
    /// so repeated attempts from the same source accrue toward the rate limit.
    fn auth_failed(
        &mut self,
        request: &SipMessage,
        now: u64,
        stale: bool,
        source: std::net::SocketAddr,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        self.auth_rate_limiter.record_failure(source.ip(), now);
        self.authentication_failure_response(request, now, stale)
    }

    /// Builds a 429 Too Many Requests response with a `Retry-After` hint for a
    /// source that has exceeded the authentication failure budget.
    fn rate_limited_response(
        &self,
        request: &SipMessage,
        now: u64,
        source: std::net::IpAddr,
    ) -> Vec<AccessOutput<Gb28181Event>> {
        let retry_after = self.auth_rate_limiter.retry_after_seconds(source, now);
        vec![AccessOutput::SendResponse(build_rate_limited_response(
            request,
            retry_after,
            self.next_tag(),
        ))]
    }

    fn authentication_failure_response(
        &self,
        request: &SipMessage,
        now: u64,
        stale: bool,
    ) -> Result<Vec<AccessOutput<Gb28181Event>>, AccessError> {
        let challenge = if stale {
            self.digest_context.generate_stale_challenge(now)
        } else {
            self.digest_context.generate_challenge(now)
        }
        .map_err(|e| AccessError::Internal(e.to_string()))?;
        Ok(vec![AccessOutput::SendResponse(build_challenge_response(
            request,
            &challenge,
            self.next_tag(),
        ))])
    }
}
