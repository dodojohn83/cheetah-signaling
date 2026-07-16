//! REGISTER handling for the GB28181 state machine.

use crate::{Gb28181ModuleError, device_id::validate_device_id, output::*};
use cheetah_gb28181_core::{DigestResponse, HeaderName, Method, SipHeaders, SipMessage};
use cheetah_signal_types::UtcTimestamp;
use std::net::SocketAddr;

use super::{Gb28181Module, message::ok_response, message::unauthorized_response};

impl Gb28181Module {
    /// Handles a REGISTER request: challenge unauthenticated requests, validate
    /// digest responses, and emit registration/refresh/deregister outputs.
    pub(super) fn handle_register(
        &mut self,
        source: SocketAddr,
        message: SipMessage,
        now: UtcTimestamp,
    ) -> Result<Vec<Gb28181Output>, Gb28181ModuleError> {
        let SipMessage::Request { line, headers, .. } = &message else {
            return Err(Gb28181ModuleError::InvalidMessage(
                "expected REGISTER request".into(),
            ));
        };

        let external_id = extract_from_user(headers)
            .map(validate_device_id)
            .ok_or_else(|| Gb28181ModuleError::InvalidMessage("missing From user".into()))??;

        let expires = parse_expires(headers).unwrap_or(3600);
        let call_id = message.call_id().unwrap_or("").to_string();
        let from_tag = parse_from_tag(headers).unwrap_or_default();
        let user_agent = headers
            .get(&HeaderName::UserAgent)
            .map(|v| v.as_str().to_string());

        if let Some(auth) = headers.get(&HeaderName::Authorization) {
            let response = DigestResponse::parse(auth.as_str())?;
            let password = self
                .config
                .auth_policy
                .password_lookup
                .password_for(&external_id, &self.config.realm)
                .ok_or(Gb28181ModuleError::Unauthorized)?;
            let uri = line.uri.encode();
            let now_sec = now.as_unix_seconds() as u64;
            if self
                .digest
                .validate(
                    &response,
                    &Method::Register,
                    &uri,
                    &password,
                    &mut self.replay_cache,
                    now_sec,
                )
                .is_err()
            {
                let challenge = self.digest.generate_stale_challenge(now_sec)?;
                return Ok(vec![Gb28181Output::SendMessage {
                    endpoint: source,
                    message: unauthorized_response(&message, &challenge.to_header_value()),
                }]);
            }

            if expires == 0 {
                self.registration = None;
                return Ok(vec![
                    Gb28181Output::Deregister,
                    Gb28181Output::SendMessage {
                        endpoint: source,
                        message: ok_response(&message),
                    },
                ]);
            }

            let is_new = self.registration.is_none();
            self.registration = Some(super::Registration {
                external_id: external_id.clone(),
                endpoint: source,
                call_id,
                from_tag,
                expires_seconds: expires,
                authenticated: true,
            });

            let mut outputs = Vec::new();
            if is_new {
                outputs.push(Gb28181Output::Register(Gb28181Register {
                    external_id: external_id.clone(),
                    realm: self.config.realm.clone(),
                    name: user_agent,
                    manufacturer: None,
                    model: None,
                    firmware: None,
                    endpoint: source,
                    expires_seconds: expires,
                    registered_at: now,
                }));
            } else {
                outputs.push(Gb28181Output::Refresh(Gb28181Refresh {
                    external_id: external_id.clone(),
                    endpoint: source,
                    expires_seconds: expires,
                    refreshed_at: now,
                }));
            }
            outputs.push(Gb28181Output::SendMessage {
                endpoint: source,
                message: ok_response(&message),
            });
            Ok(outputs)
        } else {
            let now_sec = now.as_unix_seconds() as u64;
            let challenge = self.digest.generate_challenge(now_sec)?;
            Ok(vec![Gb28181Output::SendMessage {
                endpoint: source,
                message: unauthorized_response(&message, &challenge.to_header_value()),
            }])
        }
    }
}

pub(super) fn extract_from_user(headers: &SipHeaders) -> Option<&str> {
    headers
        .get(&HeaderName::From)
        .and_then(|v| extract_sip_user(v.as_str()))
}

fn extract_sip_user(value: &str) -> Option<&str> {
    let scheme_pos = value.find("sip:").or_else(|| value.find("sips:"))?;
    let is_sips = value[scheme_pos..].starts_with("sips:");
    let scheme_len = if is_sips { 5 } else { 4 };
    let after_scheme = &value[scheme_pos + scheme_len..];
    let at = after_scheme.find('@')?;
    let user = &after_scheme[..at];
    let user = user
        .trim_start_matches(|c: char| c == '<' || c == '"' || c.is_whitespace())
        .trim_end_matches(|c: char| c == '>' || c == ';' || c == '"' || c.is_whitespace());
    Some(user)
}

pub(super) fn parse_from_tag(headers: &SipHeaders) -> Option<String> {
    headers.get(&HeaderName::From).and_then(|v| {
        v.as_str().split(';').find_map(|part| {
            let part = part.trim();
            part.strip_prefix("tag=")
                .map(|t| t.trim_matches('"').to_string())
        })
    })
}

pub(super) fn parse_expires(headers: &SipHeaders) -> Option<u32> {
    headers
        .get(&HeaderName::Expires)
        .and_then(|v| v.as_str().trim().parse().ok())
        .or_else(|| {
            headers.get(&HeaderName::Contact).and_then(|v| {
                v.as_str().split(';').find_map(|part| {
                    let part = part.trim();
                    part.strip_prefix("expires=").and_then(|s| s.parse().ok())
                })
            })
        })
}
