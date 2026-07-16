//! Incoming SIP request handling for the upstream cascade state machine.

use cheetah_gb28181_core::{HeaderName, Method, SipMessage};

use super::catalog::{
    CatalogQuery, build_bad_request_response, build_catalog_pages, build_ok_response,
    build_response, request_from_matches_upstream,
};
use super::{CascadeCredentialProvider, CascadeOutput, Gb28181Cascade, State};
use crate::xml::catalog::parse_catalog_query;

fn request_to_tag_matches(request: &SipMessage, expected_tag: &str) -> bool {
    let SipMessage::Request { headers, .. } = request else {
        return false;
    };
    let Some(to) = headers.get(&HeaderName::To) else {
        return false;
    };
    to.as_str()
        .split(';')
        .find_map(|param| param.trim().strip_prefix("tag="))
        .map(|tag| tag.trim() == expected_tag)
        .unwrap_or(false)
}

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

    // Only answer catalog queries from the configured upstream platform and only
    // while registered. Requests from other sources are rejected with a 403 so
    // the transaction terminates without disclosing catalog data.
    let Some(call_id) = headers.get(&HeaderName::CallId) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            403,
            "Forbidden",
            &response_tag,
            Vec::new(),
        ))];
    };

    let (registered_call_id, registered_local_tag) = match &cascade.state {
        State::Registered(r) => (r.call_id.clone(), r.local_tag.clone()),
        _ => {
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                403,
                "Forbidden",
                &response_tag,
                Vec::new(),
            ))];
        }
    };

    if registered_call_id != call_id.as_str()
        || !request_from_matches_upstream(&msg, &cascade.config.upstream)
        || !request_to_tag_matches(&msg, &registered_local_tag)
    {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            403,
            "Forbidden",
            &response_tag,
            Vec::new(),
        ))];
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

    let catalog_query = CatalogQuery {
        sn: query.sn,
        device_id: query.device_id,
        filter: cascade.config.catalog_filter.clone(),
    };

    let max_per_packet = cascade.config.catalog_max_items_per_packet as usize;
    let platform_id = cascade.platform_id().to_string();
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
            let mut outputs = Vec::with_capacity(messages.len() + 1);
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
