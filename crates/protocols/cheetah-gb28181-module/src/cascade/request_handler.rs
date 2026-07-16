//! Incoming SIP request handling for the upstream cascade state machine.

use cheetah_gb28181_core::{HeaderName, Method, SipMessage};

use super::catalog::{
    CatalogQuery, build_bad_request_response, build_catalog_pages, build_ok_response,
    build_response, request_from_matches_upstream,
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
