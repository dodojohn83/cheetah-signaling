//! Upstream play bridge handling for the GB28181 cascade state machine.
//!
//! The cascade module is only responsible for the *upstream* SIP dialog. It
//! emits `Gb28181Event::CascadePlayRequested` and waits for the application
//! layer to allocate a `MediaSession`/`MediaBinding` and prepare an SDP answer.
//! When the application signals readiness or failure, the cascade sends the
//! corresponding final response to the upstream platform.

use cheetah_gb28181_core::sdp::{SdpParserConfig, parse_sdp};
use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage, SipUri, StatusLine,
};
use std::collections::btree_map::Entry;

use super::{CascadeError, CascadeOutput, Gb28181Cascade, State, validate_token};
use crate::events::Gb28181Event;

/// SDP parser limits for bodies received from an upstream cascade platform.
const UPSTREAM_SDP_CONFIG: SdpParserConfig = SdpParserConfig {
    max_lines: 256,
    max_line_len: 1024,
    max_size: 16 * 1024,
    max_media: 4,
    max_attributes: 64,
    max_unknown_attributes: 32,
};

/// Lifecycle of a single upstream play bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BridgeState {
    /// INVITE received; `100 Trying` sent and `CascadePlayRequested` emitted.
    Invited,
    /// `200 OK` with answer SDP sent; waiting for ACK.
    Accepted,
    /// ACK received; dialog is active.
    Active,
    /// The bridge is being torn down and no new responses should be sent.
    Closing,
}

/// State for an upstream play bridge.
#[derive(Clone, Debug)]
pub(crate) struct Bridge {
    bridge_id: String,
    upstream_call_id: String,
    upstream_from: String,
    upstream_to: String,
    upstream_via: Vec<String>,
    upstream_cseq: u32,
    state: BridgeState,
    expires_at: u64,
}

pub(crate) fn handle_request<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let SipMessage::Request {
        line,
        headers,
        body,
        ..
    } = &msg
    else {
        return Vec::new();
    };

    // In-dialog requests must still come from the configured upstream
    // platform and target the local platform; the transaction layer will
    // handle matched/unmatched Call-IDs after that.
    if line.method != Method::Invite && !is_upstream_request(cascade, &msg) {
        return Vec::new();
    }

    match line.method {
        Method::Invite => handle_invite(cascade, now, msg.clone(), line, headers, body),
        Method::Ack => handle_ack(cascade, now, msg.clone()),
        Method::Bye => handle_bye(cascade, msg.clone()),
        Method::Cancel => handle_cancel(cascade, msg.clone()),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_invite<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    msg: SipMessage,
    line: &RequestLine,
    headers: &SipHeaders,
    body: &[u8],
) -> Vec<CascadeOutput> {
    // Only accept media requests while registered with the upstream platform.
    if !matches!(cascade.state, State::Registered(_)) {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            403,
            "Forbidden",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    }

    // Basic request semantic validation: the request must come from the
    // configured upstream and be addressed to this platform's host/domain. The
    // user part of the Request-URI/`To` is the upstream's chosen channel/device
    // ID and is intentionally not constrained here; the application layer
    // resolves it through `CascadeResourceMap`. A request that is not for this
    // platform is dropped silently rather than confirmed with an error.
    if !super::catalog::request_from_matches_upstream(&msg, &cascade.config.upstream) {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            403,
            "Forbidden",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    }
    if !request_targeted_at_local(&msg, &cascade.config.local_uri) {
        return Vec::new();
    }

    let Some(call_id) = headers.get(&HeaderName::CallId).map(|v| v.as_str()) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    };

    // Retransmitted INVITE while still invited: just re-send the provisional
    // response without re-emitting the bridge request.
    if let Some(bridge) = cascade.bridges.get(call_id) {
        if bridge.state == BridgeState::Invited {
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                100,
                "Trying",
                &cascade.next_local_tag(now),
                Vec::new(),
            ))];
        }
        // An INVITE retransmission after a final response has been sent is
        // absorbed by the transaction layer in a real UAS. We ignore it here
        // because the 200 OK/BYE state is already final.
        return Vec::new();
    }

    let Some(cseq) = msg.cseq().map(|(n, _)| n) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    };

    let target_user = match line.uri.user() {
        Some(user) if !user.is_empty() => user.to_string(),
        _ => {
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                400,
                "Bad Request",
                &cascade.next_local_tag(now),
                Vec::new(),
            ))];
        }
    };

    // Validate the remote SDP offer. We only need to parse it; the actual
    // media negotiation and answer construction are performed by the media
    // scheduler in the application layer.
    let remote_sdp = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => {
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                400,
                "Bad Request",
                &cascade.next_local_tag(now),
                Vec::new(),
            ))];
        }
    };
    if parse_sdp(remote_sdp.as_bytes(), &UPSTREAM_SDP_CONFIG).is_err() {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    }

    let platform_id = cascade.platform_id().to_string();
    if cascade.bridges.len() >= cascade.config.media_bridge_max_sessions as usize {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            486,
            "Busy Here",
            &cascade.next_local_tag(now),
            Vec::new(),
        ))];
    }

    cascade.bridge_counter += 1;
    let bridge_id = format!("{}-bridge-{}", platform_id, cascade.bridge_counter);
    let _ = validate_token(&bridge_id);

    let upstream_via: Vec<String> = headers
        .get_all(&HeaderName::Via)
        .map(|v| v.as_str().to_string())
        .collect();
    let timeout = cascade.config.media_bridge_transaction_timeout_seconds as u64;
    let bridge = Bridge {
        bridge_id: bridge_id.clone(),
        upstream_call_id: call_id.to_string(),
        upstream_from: headers
            .get(&HeaderName::From)
            .map(|v| v.as_str().to_string())
            .unwrap_or_default(),
        upstream_to: headers
            .get(&HeaderName::To)
            .map(|v| v.as_str().to_string())
            .unwrap_or_default(),
        upstream_via,
        upstream_cseq: cseq,
        state: BridgeState::Invited,
        expires_at: now.saturating_add(timeout),
    };
    cascade.bridges.insert(call_id.to_string(), bridge);

    let response_tag = cascade.next_local_tag(now);
    vec![
        CascadeOutput::SendResponse(build_response(
            &msg,
            100,
            "Trying",
            &response_tag,
            Vec::new(),
        )),
        CascadeOutput::EmitEvent(Gb28181Event::CascadePlayRequested {
            domain_id: cascade.config.domain_id.clone(),
            platform_id,
            bridge_id,
            upstream_call_id: call_id.to_string(),
            upstream_from: headers
                .get(&HeaderName::From)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default(),
            upstream_to: headers
                .get(&HeaderName::To)
                .map(|v| v.as_str().to_string())
                .unwrap_or_default(),
            target_user,
            remote_sdp,
        }),
    ]
}

fn is_upstream_request<P: super::CascadeCredentialProvider>(
    cascade: &Gb28181Cascade<P>,
    msg: &SipMessage,
) -> bool {
    request_targeted_at_local(msg, &cascade.config.local_uri)
        && super::catalog::request_from_matches_upstream(msg, &cascade.config.upstream)
}

fn request_targeted_at_local(request: &SipMessage, local: &SipUri) -> bool {
    let SipMessage::Request { line, headers, .. } = request else {
        return false;
    };
    if !line.uri.host().eq_ignore_ascii_case(local.host()) {
        return false;
    }
    let Some(to) = headers.get(&HeaderName::To) else {
        return false;
    };
    let Some(uri) = super::catalog::parse_uri_from_header(to) else {
        return false;
    };
    uri.host().eq_ignore_ascii_case(local.host())
}

fn handle_ack<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let call_id = msg.call_id().map(|s| s.to_string());
    let Some(call_id) = call_id else {
        return Vec::new();
    };
    let Some(bridge) = cascade.bridges.get_mut(&call_id) else {
        return Vec::new();
    };
    if bridge.state == BridgeState::Accepted {
        bridge.state = BridgeState::Active;
        let active = cascade.config.media_bridge_active_timeout_seconds as u64;
        bridge.expires_at = if active == 0 {
            u64::MAX
        } else {
            now.saturating_add(active)
        };
    }
    Vec::new()
}

fn handle_bye<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let call_id = msg.call_id().map(|s| s.to_string());
    let Some(call_id) = call_id else {
        return Vec::new();
    };
    let Some(bridge) = cascade.bridges.remove(&call_id) else {
        // Unknown Call-ID: do not respond, avoiding any confirmation of which
        // dialogs are active.
        return Vec::new();
    };

    let response = build_response(&msg, 200, "OK", "", Vec::new());
    let event = Gb28181Event::CascadePlayStopped {
        domain_id: cascade.config.domain_id.clone(),
        platform_id: cascade.platform_id().to_string(),
        bridge_id: bridge.bridge_id,
        reason: "upstream BYE".to_string(),
    };
    vec![
        CascadeOutput::SendResponse(response),
        CascadeOutput::EmitEvent(event),
    ]
}

fn handle_cancel<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let call_id = msg.call_id().map(|s| s.to_string());
    let Some(call_id) = call_id else {
        return Vec::new();
    };
    let Entry::Occupied(entry) = cascade.bridges.entry(call_id.clone()) else {
        // Unknown Call-ID: do not respond, avoiding any confirmation of which
        // dialogs are active.
        return Vec::new();
    };

    let bridge = entry.get();
    // CANCEL is only meaningful for an INVITE that has not yet been answered.
    if bridge.state != BridgeState::Invited {
        // Too late: final response already sent.
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            200,
            "OK",
            "",
            Vec::new(),
        ))];
    }

    let bridge = entry.remove();
    let cancel_ok = build_response(&msg, 200, "OK", "", Vec::new());

    // Build a 487 response for the original INVITE so the UAC stops
    // retransmitting. We cannot use `build_response` here because the CANCEL
    // message is not the original INVITE; the driver layer forwards the CANCEL
    // and we synthesize the INVITE response from stored bridge state.
    let invite_response = build_invite_response_from_bridge(
        &cascade.config.local_uri,
        &bridge,
        487,
        "Request Terminated",
        Vec::new(),
    );
    let event = Gb28181Event::CascadePlayStopped {
        domain_id: cascade.config.domain_id.clone(),
        platform_id: cascade.platform_id().to_string(),
        bridge_id: bridge.bridge_id,
        reason: "upstream CANCEL".to_string(),
    };
    vec![
        CascadeOutput::SendResponse(cancel_ok),
        CascadeOutput::SendResponse(invite_response),
        CascadeOutput::EmitEvent(event),
    ]
}

/// Application callback: media resources are ready; answer the upstream INVITE.
pub(crate) fn on_media_ready<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    _now: u64,
    bridge_id: String,
    answer_sdp: String,
) -> Result<Vec<CascadeOutput>, CascadeError> {
    validate_token(&bridge_id)?;
    let Some(bridge) = cascade
        .bridges
        .values_mut()
        .find(|b| b.bridge_id == bridge_id)
    else {
        // The bridge may have been cancelled/timed out while the application
        // was preparing the answer; treat that as a harmless no-op.
        return Ok(Vec::new());
    };

    if bridge.state != BridgeState::Invited {
        return Ok(Vec::new());
    }

    // Parse the answer SDP defensively; a malformed answer should not crash the
    // state machine, but we must not forward it upstream.
    if parse_sdp(answer_sdp.as_bytes(), &UPSTREAM_SDP_CONFIG).is_err() {
        return Err(CascadeError::Internal(
            "application supplied malformed answer SDP".to_string(),
        ));
    }

    bridge.state = BridgeState::Accepted;
    let response = build_invite_response_from_bridge(
        &cascade.config.local_uri,
        bridge,
        200,
        "OK",
        answer_sdp.into_bytes(),
    );
    Ok(vec![CascadeOutput::SendResponse(response)])
}

/// Application callback: the downstream side failed or hung up.
pub(crate) fn on_media_stop<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    _now: u64,
    bridge_id: String,
) -> Result<Vec<CascadeOutput>, CascadeError> {
    validate_token(&bridge_id)?;
    let Some(call_id) = cascade
        .bridges
        .iter()
        .find(|(_, b)| b.bridge_id == bridge_id)
        .map(|(call_id, _)| call_id.clone())
    else {
        return Ok(Vec::new());
    };
    let Some(mut bridge) = cascade.bridges.remove(&call_id) else {
        return Ok(Vec::new());
    };

    let outputs = match bridge.state {
        BridgeState::Invited => {
            // The application has not yet answered the INVITE; reject it.
            bridge.state = BridgeState::Closing;
            let response = build_invite_response_from_bridge(
                &cascade.config.local_uri,
                &bridge,
                486,
                "Busy Here",
                Vec::new(),
            );
            vec![
                CascadeOutput::SendResponse(response),
                CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                    domain_id: cascade.config.domain_id.clone(),
                    platform_id: cascade.platform_id().to_string(),
                    bridge_id: bridge.bridge_id,
                    reason: "downstream media stop".to_string(),
                }),
            ]
        }
        BridgeState::Active => {
            // The dialog is established; send BYE to the upstream platform.
            bridge.state = BridgeState::Closing;
            let cseq = cascade.next_cseq();
            let branch = cascade.next_branch(&bridge.upstream_call_id, cseq);
            let request = build_bye_from_bridge(&cascade.config.local_uri, &bridge, cseq, &branch)?;
            vec![
                CascadeOutput::SendRequest(request),
                CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                    domain_id: cascade.config.domain_id.clone(),
                    platform_id: cascade.platform_id().to_string(),
                    bridge_id: bridge.bridge_id,
                    reason: "downstream media stop".to_string(),
                }),
            ]
        }
        BridgeState::Accepted | BridgeState::Closing => {
            // 200 OK already sent but ACK not seen, or already closing: just
            // clean up and notify the application.
            bridge.state = BridgeState::Closing;
            vec![CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                domain_id: cascade.config.domain_id.clone(),
                platform_id: cascade.platform_id().to_string(),
                bridge_id: bridge.bridge_id,
                reason: "downstream media stop".to_string(),
            })]
        }
    };
    Ok(outputs)
}

/// Helper to build an upstream SIP response, reusing the catalog response builder
/// for non-SDP responses.
fn build_response(
    request: &SipMessage,
    code: u16,
    reason: &str,
    response_tag: &str,
    body: Body,
) -> SipMessage {
    super::catalog::build_response(request, code, reason, response_tag, body)
}

/// Builds a response to the original upstream INVITE from stored bridge state.
fn build_invite_response_from_bridge(
    local_uri: &SipUri,
    bridge: &Bridge,
    code: u16,
    reason: &str,
    body: Body,
) -> SipMessage {
    let mut headers = SipHeaders::new();
    // Via/From/To/Call-ID/CSeq are reconstructed because the original request is
    // not available when the application callback arrives.
    for via in &bridge.upstream_via {
        headers.append(HeaderName::Via, HeaderValue::new(via.clone()));
    }
    headers.append(
        HeaderName::From,
        HeaderValue::new(bridge.upstream_from.clone()),
    );
    let to = if bridge.upstream_to.contains("tag=") {
        bridge.upstream_to.clone()
    } else {
        // The stored To did not yet have a tag; add one. The actual tag value
        // does not matter for a synthetic 487, but for a 200 OK it must be the
        // tag used in the original 100/180/200 responses. The application path
        // uses `on_media_ready` immediately, so we use the bridge_id as a
        // stable tag.
        format!("{};tag={}", bridge.upstream_to.trim(), bridge.bridge_id)
    };
    headers.append(HeaderName::To, HeaderValue::new(to));
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(bridge.upstream_call_id.clone()),
    );
    headers.append(
        HeaderName::CSeq,
        HeaderValue::new(format!("{} {}", bridge.upstream_cseq, Method::Invite)),
    );
    if !body.is_empty() {
        headers.append(HeaderName::ContentType, HeaderValue::new("application/sdp"));
    }
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    SipMessage::Response {
        line: StatusLine::new(code, reason),
        headers,
        body,
    }
}

fn build_bye_from_bridge(
    local_uri: &SipUri,
    bridge: &Bridge,
    cseq: u32,
    branch: &str,
) -> Result<SipMessage, CascadeError> {
    validate_token(branch)?;
    let from_value = HeaderValue::new(&bridge.upstream_from);
    let target = super::catalog::parse_uri_from_header(&from_value).ok_or_else(|| {
        CascadeError::Internal("cannot parse upstream From URI for BYE".to_string())
    })?;
    let remote_tag = extract_tag(&bridge.upstream_from).unwrap_or_default();

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via(
            "UDP",
            local_uri.host(),
            local_uri.port().unwrap_or(5060),
            branch,
        )?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::new(format!("<{}>;tag={}", local_uri.encode(), bridge.bridge_id)),
    );
    headers.append(
        HeaderName::To,
        HeaderValue::new(format!("<{}>;tag={}", target.encode(), remote_tag)),
    );
    headers.append(
        HeaderName::CallId,
        HeaderValue::new(bridge.upstream_call_id.clone()),
    );
    headers.append(HeaderName::CSeq, HeaderValue::new(format!("{cseq} BYE")));
    headers.append(
        HeaderName::Contact,
        HeaderValue::new(format!("<{}>", local_uri.encode())),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Bye, target),
        headers,
        body: Vec::new(),
    })
}

fn extract_tag(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let start = lower.find(";tag=")? + 5;
    let rest = &value[start..];
    let end = rest
        .find(|c: char| c == ';' || c == '<' || c == '>' || c.is_whitespace())
        .unwrap_or(rest.len());
    let tag = rest[..end].trim_matches('"');
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Removes abandoned bridges whose deadlines have expired and emits the
/// appropriate cleanup responses.
pub(crate) fn on_tick<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
) -> Result<Vec<CascadeOutput>, CascadeError> {
    let mut outputs = Vec::new();

    // Snapshot expired bridges so we can call cascade helpers while building
    // outputs without keeping a borrow on the bridge map.
    let expired: Vec<(String, Bridge)> = cascade
        .bridges
        .iter()
        .filter(|(_, b)| now >= b.expires_at)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    for (call_id, bridge) in expired {
        match bridge.state {
            BridgeState::Invited => {
                let response = build_invite_response_from_bridge(
                    &cascade.config.local_uri,
                    &bridge,
                    487,
                    "Request Terminated",
                    Vec::new(),
                );
                outputs.push(CascadeOutput::SendResponse(response));
                outputs.push(CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                    domain_id: cascade.config.domain_id.clone(),
                    platform_id: cascade.platform_id().to_string(),
                    bridge_id: bridge.bridge_id.clone(),
                    reason: "transaction timeout".to_string(),
                }));
                cascade.bridges.remove(&call_id);
            }
            BridgeState::Accepted => {
                // 200 OK was sent but no ACK arrived. The upstream will time
                // out its transaction on its own; just clean up locally and
                // notify the application.
                outputs.push(CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                    domain_id: cascade.config.domain_id.clone(),
                    platform_id: cascade.platform_id().to_string(),
                    bridge_id: bridge.bridge_id.clone(),
                    reason: "transaction timeout".to_string(),
                }));
                cascade.bridges.remove(&call_id);
            }
            BridgeState::Active => {
                let active = cascade.config.media_bridge_active_timeout_seconds as u64;
                if active > 0 && now >= bridge.expires_at {
                    let cseq = cascade.next_cseq();
                    let branch = cascade.next_branch(&bridge.upstream_call_id, cseq);
                    let request =
                        build_bye_from_bridge(&cascade.config.local_uri, &bridge, cseq, &branch)?;
                    outputs.push(CascadeOutput::SendRequest(request));
                    outputs.push(CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                        domain_id: cascade.config.domain_id.clone(),
                        platform_id: cascade.platform_id().to_string(),
                        bridge_id: bridge.bridge_id.clone(),
                        reason: "active timeout".to_string(),
                    }));
                    cascade.bridges.remove(&call_id);
                }
            }
            BridgeState::Closing => {
                outputs.push(CascadeOutput::EmitEvent(Gb28181Event::CascadePlayStopped {
                    domain_id: cascade.config.domain_id.clone(),
                    platform_id: cascade.platform_id().to_string(),
                    bridge_id: bridge.bridge_id.clone(),
                    reason: "cleanup".to_string(),
                }));
                cascade.bridges.remove(&call_id);
            }
        }
    }

    Ok(outputs)
}
