//! Sans-I/O GB28181 access state machine.

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
    DigestContext, DigestQop, DigestReplayCache, HeaderName, Method, SipMessage,
};
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

mod parse;
mod response;

use parse::{
    device_id_from_request, parse_authorization, parse_contact_header, parse_expires_header,
    resolve_expires,
};
use response::{
    build_challenge_response, build_error_response, build_message_response, build_success_response,
};

/// An input to the GB28181 access module.
#[derive(Clone)]
pub struct AccessInput {
    /// Source address of the message.
    pub source: SocketAddr,
    /// Monotonic second counter used for nonce TTL and replay windows.
    pub now: u64,
    /// Parsed SIP message.
    pub message: SipMessage,
}

impl std::fmt::Debug for AccessInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessInput")
            .field("source", &self.source)
            .field("now", &self.now)
            .field("message", &"[REDACTED]")
            .finish()
    }
}

/// An output from the GB28181 access module.
#[derive(Clone)]
pub enum AccessOutput {
    /// Send a SIP response to the transport.
    SendResponse(SipMessage),
    /// Emit a domain event for downstream consumers.
    EmitEvent(Gb28181Event),
}

impl std::fmt::Debug for AccessOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccessOutput::SendResponse(_) => {
                f.debug_tuple("SendResponse").field(&"[REDACTED]").finish()
            }
            AccessOutput::EmitEvent(event) => f.debug_tuple("EmitEvent").field(event).finish(),
        }
    }
}

/// Sans-I/O state machine for GB28181 device access.
pub struct Gb28181Access<P: CredentialProvider> {
    config: Gb28181DomainConfig,
    digest_context: DigestContext,
    replay_cache: DigestReplayCache,
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
        Ok(Self {
            config,
            digest_context: ctx,
            replay_cache: DigestReplayCache::new(1024),
            credential_provider,
            tag_counter: AtomicU64::new(1),
            registrations: RegistrationTable::new(max_registrations),
        })
    }

    /// Processes a single SIP message and returns the ordered outputs.
    pub fn process(&mut self, input: AccessInput) -> Result<Vec<AccessOutput>, AccessError> {
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

    fn process_register(&mut self, input: AccessInput) -> Result<Vec<AccessOutput>, AccessError> {
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
        // validate them; otherwise fall through to unauthenticated acceptance.
        let mut authenticated = false;
        if let Some(auth_header) = headers.get(&HeaderName::Authorization) {
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
                    Err(_) => return self.authentication_failure_response(&message, now, false),
                };
                let password = match self.credential_provider.password_for(&device_id) {
                    Some(p) => p,
                    None => return self.authentication_failure_response(&message, now, false),
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
                    Ok(()) => authenticated = true,
                    Err(cheetah_gb28181_core::DigestError::StaleNonce) => {
                        return self.authentication_failure_response(&message, now, true);
                    }
                    Err(_) => return self.authentication_failure_response(&message, now, false),
                }
            } else if let Some(password) = self.credential_provider.password_for(&device_id)
                && let Ok(digest) = parse_authorization(auth_header.as_str())
            {
                let request_uri = line.uri.encode();
                if self
                    .digest_context
                    .validate(
                        &digest,
                        &Method::Register,
                        &request_uri,
                        &password,
                        &mut self.replay_cache,
                        now,
                    )
                    .is_ok()
                {
                    authenticated = true;
                }
            }
        }

        if authenticated || self.config.auth_policy() == AuthPolicy::ChallengeOptional {
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
        source: SocketAddr,
        user_agent: Option<String>,
        now: u64,
    ) -> Result<Vec<AccessOutput>, AccessError> {
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
            if let Err(e) = self.registrations.upsert(
                device_id.clone(),
                source,
                contact.clone(),
                expires,
                now,
                user_agent.clone(),
            ) {
                return if matches!(e, AccessError::RegistrationTableFull) {
                    Ok(vec![AccessOutput::SendResponse(build_error_response(
                        message,
                        503,
                        "Service Unavailable",
                        self.next_tag(),
                    ))])
                } else {
                    Err(e)
                };
            }
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
                }),
            ])
        }
    }

    fn process_message(&mut self, input: AccessInput) -> Result<Vec<AccessOutput>, AccessError> {
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
        source: SocketAddr,
        now: u64,
        message: &SipMessage,
    ) -> Result<Vec<AccessOutput>, AccessError> {
        let SipMessage::Request { headers, body, .. } = message else {
            return Err(AccessError::Internal("expected request".to_string()));
        };

        let from = headers
            .get(&HeaderName::From)
            .map(|v| v.as_str())
            .ok_or(AccessError::InvalidDeviceId)?;
        let from_device_id =
            parse::device_from_address(from).ok_or(AccessError::InvalidDeviceId)?;

        // Reject business messages from unregistered devices before doing any
        // expensive XML parsing.
        if !self.registrations.is_registered(&from_device_id) {
            return Err(AccessError::NotRegistered);
        }

        let root = parse_xml(body, &XmlLimits::default())?;
        let cmd_type = root
            .child_text("CmdType")
            .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
        let xml_device_id = root.child_text("DeviceID").unwrap_or_default();

        if from_device_id.as_ref() != xml_device_id {
            return Err(AccessError::InvalidDeviceId);
        }

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
                    device_id: from_device_id.clone(),
                    source,
                    status: keepalive.status,
                }
            }
            "Catalog" => {
                let catalog = extract_catalog(&root)?;
                Gb28181Event::CatalogReceived {
                    domain_id: domain_id.clone(),
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
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
                    device_id: from_device_id.clone(),
                    source,
                    sn: resp.sn,
                    result: resp.result,
                }
            }
            other => return Err(AccessError::UnsupportedCmdType(other.to_string())),
        };

        let Some(was_offline) = self.registrations.touch(&from_device_id, source, now) else {
            // Business messages from a device that is not currently registered
            // must not bypass the registration policy.
            return Err(AccessError::NotRegistered);
        };

        let mut outputs = Vec::with_capacity(3);
        if was_offline {
            outputs.push(AccessOutput::EmitEvent(
                Gb28181Event::DevicePresenceChanged {
                    domain_id,
                    device_id: from_device_id.clone(),
                    source,
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

    /// Advances the registration timer wheel and returns any resulting events.
    ///
    /// Should be called by the driver at regular intervals with a monotonic
    /// second counter.
    pub fn tick(&mut self, now: u64) -> Vec<AccessOutput> {
        let heartbeat_timeout = self.config.heartbeat_timeout_seconds();
        let mut outputs = Vec::new();
        let mut expired = Vec::new();

        for (device_id, reg) in self.registrations.iter_mut() {
            if now.saturating_sub(reg.registered_at) >= reg.expires as u64 {
                expired.push(device_id.clone());
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::DeviceUnregistered {
                    domain_id: self.config.domain_id().clone(),
                    device_id: device_id.clone(),
                    source: reg.source,
                }));
                continue;
            }

            if !reg.offline && now.saturating_sub(reg.last_seen) >= heartbeat_timeout {
                reg.offline = true;
                outputs.push(AccessOutput::EmitEvent(
                    Gb28181Event::DevicePresenceChanged {
                        domain_id: self.config.domain_id().clone(),
                        device_id: device_id.clone(),
                        source: reg.source,
                        presence: DevicePresence::Offline,
                    },
                ));
            }
        }

        for device_id in expired {
            self.registrations.remove(&device_id);
        }

        outputs
    }

    fn next_tag(&self) -> String {
        let n = self.tag_counter.fetch_add(1, Ordering::Relaxed);
        format!("gb{n}")
    }

    fn bad_request_response(&self, request: &SipMessage) -> Vec<AccessOutput> {
        vec![AccessOutput::SendResponse(build_error_response(
            request,
            400,
            "Bad Request",
            self.next_tag(),
        ))]
    }

    fn authentication_failure_response(
        &self,
        request: &SipMessage,
        now: u64,
        stale: bool,
    ) -> Result<Vec<AccessOutput>, AccessError> {
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
