//! Registration handling for lower GB28181 platforms.

use crate::access::{
    build_challenge_response, build_success_response, device_id_from_request, parse_authorization,
    parse_contact_header, parse_expires_header, resolve_expires,
};
use crate::config::AuthPolicy;
use crate::downstream::link::{LinkTable, PlatformLink};
use crate::downstream::{DownstreamConfig, DownstreamError, DownstreamOutput};
use crate::events::Gb28181Event;
use crate::ports::CredentialProvider;
use crate::types::DeviceId;
use cheetah_gb28181_core::{
    DigestContext, DigestReplayCache, HeaderName, Method, SipMessage, SipUri,
};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Handles an incoming `REGISTER` request from a lower platform.
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_register<P: CredentialProvider>(
    config: &DownstreamConfig,
    digest_context: &DigestContext,
    replay_cache: &mut DigestReplayCache,
    credential_provider: &P,
    links: &mut LinkTable,
    tag_counter: &AtomicU64,
    source: SocketAddr,
    now: u64,
    message: SipMessage,
) -> Result<Vec<DownstreamOutput>, DownstreamError> {
    let SipMessage::Request { line, headers, .. } = &message else {
        return Ok(Vec::new());
    };

    let platform_id = device_id_from_request(line, headers)?;
    let (contact, contact_expires) = parse_contact_header(headers)?;
    let expires_header = parse_expires_header(headers);
    let expires = resolve_expires(
        contact_expires,
        expires_header,
        config.default_expires_seconds(),
        config.max_expires_seconds(),
    );

    let user_agent = headers
        .get(&HeaderName::UserAgent)
        .map(|v| v.as_str().to_string());

    let local_tag = format!("gb{}", tag_counter.fetch_add(1, Ordering::Relaxed));

    let mut authenticated = false;
    if let Some(auth_header) = headers.get(&HeaderName::Authorization) {
        if config.auth_policy() == AuthPolicy::Required {
            let password =
                credential_provider
                    .password_for(&platform_id)
                    .ok_or(DownstreamError::Access(
                        crate::error::AccessError::AuthenticationFailed,
                    ))?;
            let digest = parse_authorization(auth_header.as_str()).map_err(|_| {
                DownstreamError::Access(crate::error::AccessError::AuthenticationFailed)
            })?;
            let request_uri = line.uri.encode();
            digest_context
                .validate(
                    &digest,
                    &Method::Register,
                    &request_uri,
                    &password,
                    replay_cache,
                    now,
                )
                .map_err(|_| {
                    DownstreamError::Access(crate::error::AccessError::AuthenticationFailed)
                })?;
            authenticated = true;
        } else if let Some(password) = credential_provider.password_for(&platform_id)
            && let Ok(digest) = parse_authorization(auth_header.as_str())
        {
            let request_uri = line.uri.encode();
            if digest_context
                .validate(
                    &digest,
                    &Method::Register,
                    &request_uri,
                    &password,
                    replay_cache,
                    now,
                )
                .is_ok()
            {
                authenticated = true;
            }
        }
    }

    if authenticated || config.auth_policy() == AuthPolicy::ChallengeOptional {
        register_accepted(
            config,
            message,
            &contact,
            expires,
            platform_id,
            source,
            user_agent,
            links,
            local_tag,
            now,
        )
    } else {
        let challenge = digest_context.generate_challenge(now).map_err(|e| {
            DownstreamError::Access(crate::error::AccessError::Internal(e.to_string()))
        })?;
        Ok(vec![DownstreamOutput::SendResponse(
            build_challenge_response(&message, &challenge, local_tag),
        )])
    }
}

#[allow(clippy::too_many_arguments)]
fn register_accepted(
    config: &DownstreamConfig,
    message: SipMessage,
    contact: &SipUri,
    expires: u32,
    platform_id: DeviceId,
    source: SocketAddr,
    user_agent: Option<String>,
    links: &mut LinkTable,
    local_tag: String,
    now: u64,
) -> Result<Vec<DownstreamOutput>, DownstreamError> {
    if expires == 0 {
        links.remove(&platform_id);
        let response = build_success_response(&message, contact, expires, local_tag);
        return Ok(vec![
            DownstreamOutput::SendResponse(response),
            DownstreamOutput::EmitEvent(Gb28181Event::DeviceUnregistered {
                domain_id: config.domain_id().clone(),
                device_id: platform_id,
                source,
            }),
        ]);
    }

    let response = build_success_response(&message, contact, expires, local_tag.clone());
    let contact_string = contact.encode();
    let link = PlatformLink {
        source,
        contact: contact.clone(),
        registered_at: now,
        expires,
        last_seen: now,
        offline: false,
        local_tag,
        next_cseq: 1,
    };
    links.upsert(platform_id.clone(), link)?;

    Ok(vec![
        DownstreamOutput::SendResponse(response),
        DownstreamOutput::EmitEvent(Gb28181Event::DeviceRegistered {
            domain_id: config.domain_id().clone(),
            device_id: platform_id,
            source,
            contact: contact_string,
            expires,
            user_agent,
        }),
    ])
}
