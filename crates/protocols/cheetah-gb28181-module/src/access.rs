//! Sans-I/O GB28181 access state machine.

use crate::config::{AuthPolicy, Gb28181DomainConfig};
use crate::error::AccessError;
use crate::events::Gb28181Event;
use crate::ports::CredentialProvider;
use crate::types::DeviceId;
use cheetah_gb28181_core::{
    DigestChallenge, DigestContext, DigestQop, DigestReplayCache, DigestResponse, HeaderName,
    HeaderValue, Method, SipHeaders, SipMessage, SipUri, StatusLine,
};
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// An input to the GB28181 access module.
#[derive(Clone, Debug)]
pub struct AccessInput {
    /// Source address of the message.
    pub source: SocketAddr,
    /// Monotonic second counter used for nonce TTL and replay windows.
    pub now: u64,
    /// Parsed SIP message.
    pub message: SipMessage,
}

/// An output from the GB28181 access module.
#[derive(Clone, Debug)]
pub enum AccessOutput {
    /// Send a SIP response to the transport.
    SendResponse(SipMessage),
    /// Emit a domain event for downstream consumers.
    EmitEvent(Gb28181Event),
}

/// Sans-I/O state machine for GB28181 device access.
pub struct Gb28181Access<P: CredentialProvider> {
    config: Gb28181DomainConfig,
    digest_context: DigestContext,
    replay_cache: DigestReplayCache,
    credential_provider: P,
    tag_counter: AtomicU64,
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
        let ctx = DigestContext::new(&config.realm, config.digest_secret.expose_secret())
            .map_err(|e| AccessError::Internal(e.to_string()))?
            .allow_md5(config.allow_md5)
            .preferred_algorithm(config.preferred_algorithm);
        let ctx = if config.auth_policy == AuthPolicy::ChallengeOptional {
            ctx.qop(None)
        } else {
            ctx.qop(Some(DigestQop::Auth))
        }
        .map_err(|e| AccessError::Internal(e.to_string()))?;
        Ok(Self {
            config,
            digest_context: ctx,
            replay_cache: DigestReplayCache::new(1024),
            credential_provider,
            tag_counter: AtomicU64::new(1),
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

        let device_id = device_id_from_request(line, headers)?;
        let (contact_uri, contact_expires) = parse_contact_header(headers)?;
        let expires_header = parse_expires_header(headers);
        let expires = resolve_expires(contact_expires, expires_header, &self.config);

        if let Some(auth_header) = headers.get(&HeaderName::Authorization) {
            let password = self
                .credential_provider
                .password_for(&device_id)
                .ok_or(AccessError::AuthenticationFailed)?;

            let digest = parse_authorization(auth_header.as_str())
                .map_err(|_| AccessError::AuthenticationFailed)?;

            let request_uri = line.uri.encode();
            self.digest_context
                .validate(
                    &digest,
                    &Method::Register,
                    &request_uri,
                    &password,
                    &mut self.replay_cache,
                    now,
                )
                .map_err(|_| AccessError::AuthenticationFailed)?;

            let user_agent = headers
                .get(&HeaderName::UserAgent)
                .map(|v| v.as_str().to_string());
            Ok(self.register_accepted(
                &message,
                &contact_uri,
                expires,
                device_id,
                source,
                user_agent,
            ))
        } else if self.config.auth_policy == AuthPolicy::ChallengeOptional {
            let user_agent = headers
                .get(&HeaderName::UserAgent)
                .map(|v| v.as_str().to_string());
            Ok(self.register_accepted(
                &message,
                &contact_uri,
                expires,
                device_id,
                source,
                user_agent,
            ))
        } else {
            let challenge = self
                .digest_context
                .generate_challenge(now)
                .map_err(|e| AccessError::Internal(e.to_string()))?;
            let response = build_challenge_response(&message, &challenge, self.next_tag());
            Ok(vec![AccessOutput::SendResponse(response)])
        }
    }

    fn register_accepted(
        &mut self,
        message: &SipMessage,
        contact_uri: &SipUri,
        expires: u32,
        device_id: DeviceId,
        source: SocketAddr,
        user_agent: Option<String>,
    ) -> Vec<AccessOutput> {
        let response = build_success_response(message, contact_uri, expires, self.next_tag());
        if expires == 0 {
            vec![
                AccessOutput::SendResponse(response),
                AccessOutput::EmitEvent(Gb28181Event::DeviceUnregistered {
                    domain_id: self.config.domain_id.clone(),
                    device_id,
                    source,
                }),
            ]
        } else {
            let contact = contact_uri.encode();
            vec![
                AccessOutput::SendResponse(response),
                AccessOutput::EmitEvent(Gb28181Event::DeviceRegistered {
                    domain_id: self.config.domain_id.clone(),
                    device_id,
                    source,
                    contact,
                    expires,
                    user_agent,
                }),
            ]
        }
    }

    fn process_message(&mut self, _input: AccessInput) -> Result<Vec<AccessOutput>, AccessError> {
        Err(AccessError::UnsupportedMethod)
    }

    fn next_tag(&self) -> String {
        let n = self.tag_counter.fetch_add(1, Ordering::Relaxed);
        format!("gb{n}")
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
    parse_address_with_expires(value).map_err(|_| AccessError::InvalidContact)
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
    let expires = params_text.split(';').find_map(|token| {
        let token = token.trim();
        token
            .strip_prefix("expires=")
            .and_then(|v| v.trim().parse::<u32>().ok())
    });
    Ok((uri, expires))
}

fn parse_expires_header(headers: &SipHeaders) -> Option<u32> {
    headers
        .get(&HeaderName::Expires)
        .and_then(|v| v.as_str().trim().parse::<u32>().ok())
}

fn resolve_expires(
    contact_expires: Option<u32>,
    header_expires: Option<u32>,
    config: &Gb28181DomainConfig,
) -> u32 {
    let requested = contact_expires
        .or(header_expires)
        .unwrap_or(config.default_expires_seconds);
    requested.clamp(0, config.max_expires_seconds)
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
        HeaderValue::new(format!("{};expires={}", contact.encode(), expires)),
    );
    headers.append(HeaderName::Expires, HeaderValue::new(expires.to_string()));
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
