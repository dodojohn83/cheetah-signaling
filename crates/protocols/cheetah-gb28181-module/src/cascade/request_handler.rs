//! Incoming SIP request handling for the upstream cascade state machine.

use cheetah_gb28181_core::{
    DigestError, DigestResponse, HeaderName, HeaderValue, Method, RequestLine, SipHeaders,
    SipMessage, StatusLine,
};
use tracing::warn;

use super::catalog::{
    CatalogQuery, build_bad_request_response, build_catalog_pages, build_ok_response,
    build_response, request_from_matches_upstream, request_target_matches_local,
    request_to_uri_matches_local,
};
use super::{CascadeCredentialProvider, CascadeOutput, Gb28181Cascade, State};
use crate::xml::catalog::parse_catalog_query;

pub(super) fn handle_request<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let Some(provider) = cascade.catalog_provider.clone() else {
        return Vec::new();
    };

    let SipMessage::Request {
        line,
        headers,
        body,
        ..
    } = &msg
    else {
        return Vec::new();
    };

    if line.method != Method::Message {
        return Vec::new();
    }

    // All final responses below need a local To-tag.
    let response_tag = cascade.next_local_tag(now);

    // Catalog queries are accepted only while the cascade is registered and the
    // request comes from the configured upstream platform. REGISTER is dialog-less
    // in SIP, so upstream platforms send Catalog queries as standalone MESSAGE
    // transactions with their own Call-ID and a fresh To tag. We therefore do NOT
    // require the query to reuse the registration Call-ID or To-tag.
    let Some(_call_id) = headers.get(&HeaderName::CallId) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    };

    if !matches!(cascade.state, State::Registered(_))
        || !request_target_matches_local(&msg, &cascade.config.local_uri)
        || !request_to_uri_matches_local(&msg, &cascade.config.local_uri)
        || !request_from_matches_upstream(&msg, &cascade.config.upstream)
    {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            403,
            "Forbidden",
            &response_tag,
            Vec::new(),
        ))];
    }

    match check_catalog_authorization(cascade, &msg, line, headers, now, &response_tag) {
        Ok(Some(response)) => {
            return vec![CascadeOutput::SendResponse(response)];
        }
        Err(e) => {
            warn!(error = %e, "catalog authorization internal error");
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                500,
                "Server Internal Error",
                &response_tag,
                Vec::new(),
            ))];
        }
        Ok(None) => {}
    }

    let Some(content_type) = headers.get(&HeaderName::ContentType) else {
        return vec![CascadeOutput::SendResponse(build_ok_response(
            &msg,
            &response_tag,
        ))];
    };

    if !content_type.as_str().contains("MANSCDP") && !content_type.as_str().contains("xml") {
        return vec![CascadeOutput::SendResponse(build_ok_response(
            &msg,
            &response_tag,
        ))];
    }

    let query = match parse_catalog_query(body) {
        Ok(q) => q,
        Err(_) => {
            return vec![CascadeOutput::SendResponse(build_bad_request_response(
                &msg,
                "Bad Request",
                &response_tag,
            ))];
        }
    };

    let platform_id = cascade.platform_id().to_string();
    if query.device_id != platform_id {
        return vec![CascadeOutput::SendResponse(build_bad_request_response(
            &msg,
            "Catalog target DeviceID mismatch",
            &response_tag,
        ))];
    }

    let catalog_query = CatalogQuery {
        sn: query.sn,
        device_id: query.device_id,
        filter: cascade.config.catalog_filter.clone(),
    };

    let max_per_packet = cascade.config.catalog_max_items_per_packet as usize;
    match build_catalog_pages(
        &cascade.config,
        &provider,
        &catalog_query,
        max_per_packet,
        now,
        &mut cascade.request_counter,
        &platform_id,
    ) {
        Ok(messages) => {
            let mut outputs = Vec::with_capacity(messages.len().saturating_add(1));
            outputs.push(CascadeOutput::SendResponse(build_ok_response(
                &msg,
                &response_tag,
            )));
            for message in messages {
                outputs.push(CascadeOutput::SendRequest(message));
            }
            outputs
        }
        Err(_) => vec![CascadeOutput::SendResponse(build_response(
            &msg,
            500,
            "Server Internal Error",
            &response_tag,
            Vec::new(),
        ))],
    }
}

/// Returns `Some(response)` when the incoming `Catalog` `MESSAGE` must be
/// challenged or rejected based on SIP Digest authentication. Returns `None` when
/// authentication is disabled or the request carries a valid `Authorization`
/// header.
pub(super) fn check_catalog_authorization<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    msg: &SipMessage,
    line: &RequestLine,
    headers: &SipHeaders,
    now: u64,
    response_tag: &str,
) -> Result<Option<SipMessage>, crate::cascade::CascadeError> {
    let Some(auth) = cascade.inbound_auth.as_ref() else {
        return Ok(None);
    };

    let credential_ref = cascade
        .config
        .catalog_inbound_digest_credential_ref
        .as_deref()
        .unwrap_or(&cascade.config.credential_ref);
    let Some(password) = cascade.provider.password_for(credential_ref) else {
        return Ok(Some(build_response(
            msg,
            403,
            "Forbidden",
            response_tag,
            Vec::new(),
        )));
    };

    let mut replay = auth.replay.lock().unwrap_or_else(|e| e.into_inner());
    let uri = line.uri.encode();

    if let Some(auth_header) = headers.get(&HeaderName::Authorization) {
        match DigestResponse::parse(auth_header.as_str()) {
            Ok(response) => {
                if auth
                    .digest
                    .validate(&response, &line.method, &uri, &password, &mut replay, now)
                    .is_ok()
                {
                    return Ok(None);
                }
            }
            Err(DigestError::Malformed(_)) | Err(DigestError::InvalidQop) => {
                // Fall through to challenge.
            }
            Err(_) => {
                // Transient/internal digest errors: challenge again.
            }
        }
    }

    let challenge = auth.digest.generate_challenge(now)?;
    let response = build_unauthorized_response(msg, response_tag, &challenge)?;
    Ok(Some(response))
}

fn build_unauthorized_response(
    request: &SipMessage,
    response_tag: &str,
    challenge: &cheetah_gb28181_core::DigestChallenge,
) -> Result<SipMessage, crate::cascade::CascadeError> {
    let SipMessage::Request { headers, .. } = request else {
        return Err(crate::cascade::CascadeError::Internal(
            "caller ensured a request".to_string(),
        ));
    };

    let mut response_headers = SipHeaders::new();
    for via in headers.get_all(&HeaderName::Via) {
        response_headers.append(HeaderName::Via, via.clone());
    }
    if let Some(from) = headers.get(&HeaderName::From) {
        response_headers.append(HeaderName::From, from.clone());
    }
    if let Some(to) = headers.get(&HeaderName::To) {
        let to_str = to.as_str();
        let has_tag = to_str
            .split(';')
            .any(|param| param.trim().starts_with("tag="));
        if has_tag {
            response_headers.append(HeaderName::To, to.clone());
        } else {
            response_headers.append(
                HeaderName::To,
                HeaderValue::new(format!("{};tag={}", to_str.trim(), response_tag)),
            );
        }
    }
    if let Some(call_id) = headers.get(&HeaderName::CallId) {
        response_headers.append(HeaderName::CallId, call_id.clone());
    }
    if let Some(cseq) = headers.get(&HeaderName::CSeq) {
        response_headers.append(HeaderName::CSeq, cseq.clone());
    }
    response_headers.append(
        HeaderName::WwwAuthenticate,
        HeaderValue::new(challenge.to_header_value()),
    );
    response_headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Response {
        line: StatusLine::new(401, "Unauthorized"),
        headers: response_headers,
        body: Vec::new(),
    })
}
