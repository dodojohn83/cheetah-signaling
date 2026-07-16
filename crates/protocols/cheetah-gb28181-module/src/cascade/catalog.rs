//! Catalog sharing support for upstream GB28181 cascade platforms.

use std::sync::Arc;

use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
};

use crate::cascade::{CascadeConfig, CascadeError, validate_token};
use crate::xml::catalog::{CatalogItem, build_catalog_response};

/// Filter applied by the catalog provider so the shared catalog never leaks
/// internal tenant IDs, node addresses, or protocol credentials.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CatalogFilter {
    /// Optional tenant boundary. Only resources belonging to this tenant are
    /// eligible for sharing.
    pub tenant_id: Option<String>,
    /// Optional whitelist of exposed device identifiers. When set, only listed
    /// IDs are returned.
    pub whitelisted_device_ids: Vec<String>,
    /// Optional set of tags; the provider returns items tagged with any of
    /// these values.
    pub tags: Vec<String>,
    /// Optional organization tree prefix. The provider returns items whose
    /// organization path starts with this value.
    pub org_path_prefix: Option<String>,
}

/// A single catalog query delivered to the provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogQuery {
    /// Sequence number from the upstream `<SN>` element.
    pub sn: String,
    /// Device identifier from the upstream `<DeviceID>` element.
    pub device_id: String,
    /// Filter derived from the cascade share policy.
    pub filter: CatalogFilter,
}

/// One page of catalog items returned by a provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogPage {
    /// Items in this page.
    pub items: Vec<CatalogItem>,
    /// Total number of items matching the query.
    pub total: usize,
    /// Opaque token for the next page, if any. Passing this back to the
    /// provider yields the following page without numeric offsets.
    pub next_cursor: Option<String>,
}

/// Errors returned by a catalog provider.
#[derive(Clone, Debug, thiserror::Error, Eq, PartialEq)]
pub enum CatalogError {
    /// An internal provider error.
    #[error("catalog provider error: {0}")]
    Internal(String),
}

/// Port trait for retrieving a filtered, paged view of the catalog that may be
/// shared with an upstream platform.
pub trait CatalogProvider: Send + Sync {
    /// Returns one page of shared catalog items and an optional opaque cursor for
    /// the next page. The cursor is passed back verbatim on the next call.
    fn query_page(
        &self,
        query: &CatalogQuery,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<CatalogPage, CatalogError>;
}

impl CatalogFilter {
    /// Returns true when the filter imposes no restrictions.
    pub fn is_permissive(&self) -> bool {
        self.tenant_id.is_none()
            && self.whitelisted_device_ids.is_empty()
            && self.tags.is_empty()
            && self.org_path_prefix.is_none()
    }
}

/// Extracts the bare URI from a header value that may contain a display name
/// and parameters (e.g. `<sip:alice@example.com>;tag=abc`).
pub(crate) fn parse_uri_from_header(value: &HeaderValue) -> Option<SipUri> {
    let value_str = value.as_str().trim();
    let uri_str = if let Some(start) = value_str.find('<') {
        let end_rel = value_str[start + 1..].find('>')?;
        let end = start + 1 + end_rel;
        &value_str[start + 1..end]
    } else if let Some(semi) = value_str.find(';') {
        &value_str[..semi]
    } else {
        value_str
    };
    SipUri::parse(uri_str.trim()).ok()
}

fn uri_matches(a: &SipUri, b: &SipUri) -> bool {
    if !a.host().eq_ignore_ascii_case(b.host()) {
        return false;
    }
    if let Some(expected_user) = b.user() {
        a.user() == Some(expected_user)
    } else {
        a.user().is_none()
    }
}

/// Returns true when the `From` header of the request identifies the
/// configured upstream platform.
pub(crate) fn request_from_matches_upstream(request: &SipMessage, upstream: &SipUri) -> bool {
    let SipMessage::Request { headers, .. } = request else {
        return false;
    };
    let Some(from) = headers.get(&HeaderName::From) else {
        return false;
    };
    let Some(uri) = parse_uri_from_header(from) else {
        return false;
    };
    uri_matches(&uri, upstream)
}

/// Returns true when the `To` header (ignoring any tag) identifies the local
/// platform AOR.
pub(crate) fn request_to_uri_matches_local(request: &SipMessage, local: &SipUri) -> bool {
    let SipMessage::Request { headers, .. } = request else {
        return false;
    };
    let Some(to) = headers.get(&HeaderName::To) else {
        return false;
    };
    let Some(uri) = parse_uri_from_header(to) else {
        return false;
    };
    uri_matches(&uri, local)
}

/// Returns true when the request-line target URI identifies the local platform.
pub(crate) fn request_target_matches_local(request: &SipMessage, local: &SipUri) -> bool {
    let SipMessage::Request { line, .. } = request else {
        return false;
    };
    uri_matches(&line.uri, local)
}

/// Builds a `200 OK` response for an incoming SIP request.
pub(crate) fn build_ok_response(request: &SipMessage, response_tag: &str) -> SipMessage {
    build_response(request, 200, "OK", response_tag, Vec::new())
}

/// Builds a `400 Bad Request` response for an incoming SIP request.
pub(crate) fn build_bad_request_response(
    request: &SipMessage,
    reason: &str,
    response_tag: &str,
) -> SipMessage {
    build_response(request, 400, reason, response_tag, Vec::new())
}

pub(crate) fn build_response(
    request: &SipMessage,
    code: u16,
    reason: &str,
    response_tag: &str,
    body: Body,
) -> SipMessage {
    let SipMessage::Request { headers, .. } = request else {
        unreachable!("caller ensures a request");
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
        if has_tag || response_tag.is_empty() {
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
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers: response_headers,
        body,
    }
}

/// Builds a `MESSAGE` request carrying a `Catalog` response XML payload.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_catalog_message(
    config: &CascadeConfig,
    call_id: &str,
    cseq: u32,
    local_tag: &str,
    branch: &str,
    catalog_xml: &str,
) -> Result<SipMessage, CascadeError> {
    validate_token(call_id)?;
    validate_token(local_tag)?;
    validate_token(branch)?;

    let body: Body = catalog_xml.as_bytes().to_vec();
    let mut headers = SipHeaders::new();
    let local_host = config.local_uri.host();
    let local_port = config.local_uri.port().unwrap_or(5060);
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", local_host, local_port, branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&config.local_uri, local_tag)?,
    );
    headers.append(HeaderName::To, HeaderValue::to_uri(&config.upstream));
    headers.append(HeaderName::CallId, HeaderValue::new(call_id.to_string()));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Message));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    if let Some(ua) = &config.user_agent {
        headers.append(HeaderName::UserAgent, HeaderValue::new(ua.clone()));
    }
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, config.upstream.clone()),
        headers,
        body,
    })
}

/// Builds a sequence of catalog `MESSAGE` requests for one upstream query.
pub(crate) fn build_catalog_pages(
    config: &CascadeConfig,
    provider: &Arc<dyn CatalogProvider>,
    query: &CatalogQuery,
    max_per_packet: usize,
    now: u64,
    request_counter: &mut u64,
    platform_id: &str,
) -> Result<Vec<SipMessage>, CascadeError> {
    let mut messages = Vec::new();
    let mut cursor: Option<String> = None;
    let mut pages_emitted = 0;

    // Independent upper bound so a misbehaving or concurrently-growing provider
    // cannot make this loop run unbounded.
    let max_per_packet = max_per_packet.max(1);
    let max_pages = (config.catalog_max_query_pages as usize).max(1);
    let max_total_items = max_per_packet.saturating_mul(max_pages);

    loop {
        if pages_emitted >= max_pages {
            break;
        }

        let page = provider
            .query_page(query, cursor.as_deref(), max_per_packet)
            .map_err(|e| CascadeError::Internal(e.to_string()))?;
        pages_emitted += 1;

        // Clamp the advertised total to the configured cap. This also protects
        // against a provider that reports a total smaller than what it returns,
        // because the cursor is exhausted when the provider returns no page.
        let total = page.total.min(max_total_items);

        if page.items.is_empty() {
            if cursor.is_none() {
                // No items at all; still emit one empty response so the
                // upstream receives a valid `SumNum=0`.
                *request_counter += 1;
                let call_id = format!("{platform_id}-{now}-{request_counter}");
                let cseq = 0;
                let local_tag = format!("{platform_id}-{now}-{request_counter}");
                let branch = format!("z9hG4bK-{call_id}-{cseq}-{request_counter}");
                let xml = build_catalog_response(&query.sn, &query.device_id, 0, &[])
                    .map_err(|e| CascadeError::Internal(format!("XML encode failed: {e}")))?;
                messages.push(build_catalog_message(
                    config, &call_id, cseq, &local_tag, &branch, &xml,
                )?);
            }
            break;
        }

        let sum_num = total.try_into().unwrap_or(u32::MAX);

        *request_counter += 1;
        let call_id = format!("{platform_id}-{now}-{request_counter}");
        let cseq = 0;
        let local_tag = format!("{platform_id}-{now}-{request_counter}");
        let branch = format!("z9hG4bK-{call_id}-{cseq}-{request_counter}");
        let xml = build_catalog_response(&query.sn, &query.device_id, sum_num, &page.items)
            .map_err(|e| CascadeError::Internal(format!("XML encode failed: {e}")))?;
        messages.push(build_catalog_message(
            config, &call_id, cseq, &local_tag, &branch, &xml,
        )?);

        if page.next_cursor.is_none() || pages_emitted >= max_pages {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(messages)
}
