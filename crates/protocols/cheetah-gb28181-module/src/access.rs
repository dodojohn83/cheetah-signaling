//! Sans-I/O GB28181 access state machine.

use crate::config::{AuthPolicy, Gb28181DomainConfig};
use crate::error::AccessError;
use crate::events::{DevicePresence, Gb28181Event};
use crate::ports::CredentialProvider;
use crate::registration::RegistrationTable;
use crate::types::DeviceId;
use crate::xml::{
    XmlLimits, parse_alarm, parse_catalog, parse_device_control_response, parse_device_info,
    parse_device_status, parse_keepalive, parse_mobile_position, parse_record_info, parse_xml,
};
use cheetah_gb28181_core::{
    DigestChallenge, DigestContext, DigestQop, DigestReplayCache, DigestResponse, HeaderName,
    HeaderValue, Method, SipHeaders, SipMessage, SipUri, StatusLine,
};
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

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
            SipMessage::Request { .. } => Err(AccessError::UnsupportedMethod),
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
                let password = match self.credential_provider.password_for(&device_id) {
                    Some(p) => p,
                    None => return self.authentication_failure_response(&message, now),
                };
                let digest = match parse_authorization(auth_header.as_str()) {
                    Ok(d) => d,
                    Err(cheetah_gb28181_core::DigestError::Malformed(_)) => {
                        return Ok(self.bad_request_response(&message));
                    }
                    Err(_) => return self.authentication_failure_response(&message, now),
                };
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
                    .is_err()
                {
                    return self.authentication_failure_response(&message, now);
                }
                authenticated = true;
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
            let challenge = self
                .digest_context
                .generate_challenge(now)
                .map_err(|e| AccessError::Internal(e.to_string()))?;
            let response = build_challenge_response(&message, &challenge, self.next_tag());
            Ok(vec![AccessOutput::SendResponse(response)])
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn register_accepted(
        &mut self,
        message: &SipMessage,
        contact_uri: &SipUri,
        expires: u32,
        device_id: DeviceId,
        source: SocketAddr,
        user_agent: Option<String>,
        now: u64,
    ) -> Result<Vec<AccessOutput>, AccessError> {
        let response = build_success_response(message, contact_uri, expires, self.next_tag());
        if expires == 0 {
            self.registrations.remove(&device_id);
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
            self.registrations.upsert(
                device_id.clone(),
                source,
                contact.clone(),
                expires,
                now,
                user_agent.clone(),
            )?;
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
        let SipMessage::Request { headers, body, .. } = &message else {
            return Err(AccessError::Internal("expected request".to_string()));
        };

        let from = headers
            .get(&HeaderName::From)
            .map(|v| v.as_str())
            .ok_or(AccessError::InvalidDeviceId)?;
        let from_device_id = device_from_address(from).ok_or(AccessError::InvalidDeviceId)?;

        let root = parse_xml(body, &XmlLimits::default())?;
        let cmd_type = root
            .child_text("CmdType")
            .ok_or_else(|| AccessError::InvalidXml("missing CmdType".to_string()))?;
        let xml_device_id = root.child_text("DeviceID").unwrap_or_default();

        if from_device_id.as_ref() != xml_device_id {
            return Err(AccessError::InvalidDeviceId);
        }
        let device_id = DeviceId::new(&xml_device_id).ok_or(AccessError::InvalidDeviceId)?;

        let Some(was_offline) = self.registrations.touch(&device_id, source, now) else {
            // Business messages from a device that is not currently registered
            // must not bypass the registration policy.
            return Err(AccessError::NotRegistered);
        };

        let mut outputs = Vec::with_capacity(3);
        if was_offline {
            outputs.push(AccessOutput::EmitEvent(
                Gb28181Event::DevicePresenceChanged {
                    domain_id: self.config.domain_id().clone(),
                    device_id: device_id.clone(),
                    source,
                    presence: DevicePresence::Online,
                },
            ));
        }

        match cmd_type.as_str() {
            "Keepalive" => {
                let keepalive = parse_keepalive(body)?;
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::Keepalive {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    status: keepalive.status,
                }));
            }
            "Catalog" => {
                let catalog = parse_catalog(body)?;
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::CatalogReceived {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    sn: catalog.sn,
                    sum_num: catalog.sum_num,
                    num: catalog.num,
                    items: catalog.items,
                }));
            }
            "DeviceInfo" => {
                let info = parse_device_info(body)?;
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::DeviceInfoReceived {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    sn: info.sn,
                    result: info.result,
                    manufacturer: info.manufacturer,
                    model: info.model,
                    firmware: info.firmware,
                }));
            }
            "DeviceStatus" => {
                let status = parse_device_status(body)?;
                outputs.push(AccessOutput::EmitEvent(
                    Gb28181Event::DeviceStatusReceived {
                        domain_id: self.config.domain_id().clone(),
                        device_id,
                        source,
                        sn: status.sn,
                        result: status.result,
                        online: status.online,
                        status: status.status,
                        reason: status.reason,
                        invalid_equip: status.invalid_equip,
                    },
                ));
            }
            "Alarm" => {
                let alarm = parse_alarm(body)?;
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::AlarmReceived {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    sn: alarm.sn,
                    priority: alarm.priority,
                    method: alarm.method,
                    alarm_type: alarm.alarm_type,
                    time: alarm.time,
                    info: alarm.info,
                }));
            }
            "MobilePosition" => {
                let pos = parse_mobile_position(body)?;
                outputs.push(AccessOutput::EmitEvent(
                    Gb28181Event::MobilePositionReceived {
                        domain_id: self.config.domain_id().clone(),
                        device_id,
                        source,
                        sn: pos.sn,
                        time: pos.time,
                        longitude: pos.longitude,
                        latitude: pos.latitude,
                        speed: pos.speed,
                        direction: pos.direction,
                        altitude: pos.altitude,
                    },
                ));
            }
            "RecordInfo" => {
                let info = parse_record_info(body)?;
                outputs.push(AccessOutput::EmitEvent(Gb28181Event::RecordInfoReceived {
                    domain_id: self.config.domain_id().clone(),
                    device_id,
                    source,
                    sn: info.sn,
                    name: info.name,
                    sum_num: info.sum_num,
                    num: info.num,
                    items: info.items,
                }));
            }
            "DeviceControl" => {
                let resp = parse_device_control_response(body)?;
                outputs.push(AccessOutput::EmitEvent(
                    Gb28181Event::DeviceControlResponseReceived {
                        domain_id: self.config.domain_id().clone(),
                        device_id,
                        source,
                        sn: resp.sn,
                        result: resp.result,
                    },
                ));
            }
            other => return Err(AccessError::UnsupportedCmdType(other.to_string())),
        }

        outputs.push(AccessOutput::SendResponse(build_message_response(
            &message,
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
    ) -> Result<Vec<AccessOutput>, AccessError> {
        let challenge = self
            .digest_context
            .generate_challenge(now)
            .map_err(|e| AccessError::Internal(e.to_string()))?;
        Ok(vec![AccessOutput::SendResponse(build_challenge_response(
            request,
            &challenge,
            self.next_tag(),
        ))])
    }
}

fn device_id_from_request(
    request: &cheetah_gb28181_core::RequestLine,
    headers: &SipHeaders,
) -> Result<DeviceId, AccessError> {
    if let Some(id) = request
        .uri
        .user()
        .filter(|u| !u.is_empty())
        .and_then(DeviceId::new)
    {
        return Ok(id);
    }
    if let Some(id) = headers
        .get(&HeaderName::To)
        .and_then(|v| device_from_address(v.as_str()))
    {
        return Ok(id);
    }
    if let Some(id) = headers
        .get(&HeaderName::From)
        .and_then(|v| device_from_address(v.as_str()))
    {
        return Ok(id);
    }
    Err(AccessError::InvalidDeviceId)
}

fn device_from_address(value: &str) -> Option<DeviceId> {
    let value = value.trim();
    let uri_text = if let Some(start) = value.find('<') {
        let end = value.find('>')?;
        value.get(start + 1..end)?
    } else {
        value.split(';').next()?
    };
    SipUri::parse(uri_text).ok().and_then(|u| {
        u.user()
            .filter(|u| !u.is_empty())
            .map(str::to_string)
            .and_then(DeviceId::new)
    })
}

fn parse_contact_header(headers: &SipHeaders) -> Result<(SipUri, Option<u32>), AccessError> {
    let value = headers
        .get(&HeaderName::Contact)
        .ok_or(AccessError::InvalidContact)?
        .as_str();
    parse_address_with_expires(value)
}

fn parse_address_with_expires(value: &str) -> Result<(SipUri, Option<u32>), AccessError> {
    let value = value.trim();
    let (uri_text, params_text) = if let Some(start) = value.find('<') {
        let end = value.find('>').ok_or(AccessError::InvalidContact)?;
        let uri_text = value
            .get(start + 1..end)
            .ok_or(AccessError::InvalidContact)?;
        let after = value.get(end + 1..).unwrap_or("");
        (uri_text, after.trim())
    } else {
        let parts: Vec<&str> = value.splitn(2, ';').collect();
        (parts[0].trim(), parts.get(1).copied().unwrap_or(""))
    };

    let uri = SipUri::parse(uri_text).map_err(|_| AccessError::InvalidContact)?;
    let mut expires = None;
    for token in params_text.split(';') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        if let Some(value) = token.strip_prefix("expires=") {
            let value = value.trim();
            if value.is_empty() {
                return Err(AccessError::InvalidExpires);
            }
            expires = Some(
                value
                    .parse::<u32>()
                    .map_err(|_| AccessError::InvalidExpires)?,
            );
        }
    }
    Ok((uri, expires))
}

fn parse_expires_header(headers: &SipHeaders) -> Result<Option<u32>, AccessError> {
    let Some(value) = headers.get(&HeaderName::Expires) else {
        return Ok(None);
    };
    let trimmed = value.as_str().trim();
    if trimmed.is_empty() {
        return Err(AccessError::InvalidExpires);
    }
    trimmed
        .parse::<u32>()
        .map(Some)
        .map_err(|_| AccessError::InvalidExpires)
}

fn resolve_expires(
    contact_expires: Option<u32>,
    header_expires: Option<u32>,
    config: &Gb28181DomainConfig,
) -> u32 {
    let requested = contact_expires
        .or(header_expires)
        .unwrap_or(config.default_expires_seconds());
    requested.clamp(0, config.max_expires_seconds())
}

fn parse_authorization(value: &str) -> Result<DigestResponse, cheetah_gb28181_core::DigestError> {
    DigestResponse::parse_with_limit(value, 4096)
}

fn build_challenge_response(
    request: &SipMessage,
    challenge: &DigestChallenge,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(
        HeaderName::WwwAuthenticate,
        HeaderValue::new(challenge.to_header_value()),
    );
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(401, "Unauthorized"),
        headers,
        body: Vec::new(),
    }
}

fn build_error_response(request: &SipMessage, code: u16, reason: &str, tag: String) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body: Vec::new(),
    }
}

fn build_success_response(
    request: &SipMessage,
    contact: &SipUri,
    expires: u32,
    tag: String,
) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>;expires={}", contact.encode(), expires)),
    );
    headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

fn build_message_response(request: &SipMessage, tag: String) -> SipMessage {
    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(200, "OK"),
        headers,
        body: Vec::new(),
    }
}

fn copy_common_headers(request: &SipMessage) -> SipHeaders {
    let mut headers = SipHeaders::new();
    // Via may appear multiple times (one per proxy hop); copy all of them.
    for value in request.headers().get_all(&HeaderName::Via) {
        headers.append(HeaderName::Via.clone(), value.clone());
    }
    for name in [HeaderName::From, HeaderName::CallId, HeaderName::CSeq] {
        if let Some(value) = request.headers().get(&name) {
            headers.append(name, value.clone());
        }
    }
    headers
}

fn add_or_replace_tag(value: &str, tag: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    let without_tag = value
        .split(';')
        .filter(|part| !part.trim().starts_with("tag="))
        .collect::<Vec<_>>()
        .join(";");
    if without_tag.is_empty() {
        format!("tag={tag}")
    } else {
        format!("{without_tag};tag={tag}")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use cheetah_gb28181_core::{HeaderName, HeaderValue, SipHeaders};

    #[test]
    fn parse_expires_header_rejects_non_numeric() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::Expires, HeaderValue::new("not-a-number"));
        assert!(matches!(
            parse_expires_header(&headers),
            Err(AccessError::InvalidExpires)
        ));
    }

    #[test]
    fn parse_expires_header_rejects_empty() {
        let mut headers = SipHeaders::new();
        headers.append(HeaderName::Expires, HeaderValue::new(""));
        assert!(matches!(
            parse_expires_header(&headers),
            Err(AccessError::InvalidExpires)
        ));
    }

    #[test]
    fn parse_address_with_expires_rejects_non_numeric_param() {
        let result = parse_address_with_expires("<sip:a@example.com>;expires=not-a-number");
        assert!(matches!(result, Err(AccessError::InvalidExpires)));
    }

    #[test]
    fn parse_address_with_expires_accepts_valid_param() {
        let (uri, expires) = parse_address_with_expires("<sip:a@example.com>;expires=60").unwrap();
        assert_eq!(expires, Some(60));
        assert_eq!(uri.user(), Some("a"));
    }
}
