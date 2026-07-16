//! Upstream event subscription handling for a GB28181 cascade platform.
//!
//! The cascade acts as a SIP notifier: it accepts incoming `SUBSCRIBE` requests
//! from the upstream platform for event packages such as `Catalog`, `Alarm` and
//! `MobilePosition`, responds with `200 OK`, and later sends `NOTIFY` requests
//! carrying the current state. Subscriptions are keyed by `Call-ID` and the
//! upstream `From` tag, refreshed on `SUBSCRIBE` with matching dialog, and
//! terminated by `Expires: 0` or by timer expiry.

use cheetah_gb28181_core::{HeaderName, HeaderValue, SipMessage, SipUri};

use super::catalog::{
    build_ok_response, build_response, parse_uri_from_header, request_from_matches_upstream,
    request_target_matches_local, request_to_uri_matches_local,
};
use super::{CascadeCredentialProvider, CascadeError, CascadeOutput, Gb28181Cascade, State};

/// How long a `NOTIFY` request may stay pending before it is retransmitted.
const NOTIFY_TIMEOUT_SECONDS: u64 = 5;

/// How many times a `NOTIFY` may be retransmitted before the subscription is
/// considered failed.
const NOTIFY_MAX_RETRIES: u32 = 3;

/// Event packages supported by this notifier.
const SUPPORTED_PACKAGES: &[&str] = &["Catalog", "Alarm", "MobilePosition"];

/// A single active or terminating upstream subscription.
#[derive(Clone, Debug)]
pub(crate) struct Subscription {
    call_id: String,
    local_tag: String,
    remote_tag: String,
    remote_uri: SipUri,
    event_package: String,
    expires_at: u64,
    next_cseq: u32,
    pending_notify: Option<PendingNotify>,
    last_active_at: u64,
}

/// State for a `NOTIFY` that has been sent but not yet acknowledged.
#[derive(Clone, Debug)]
struct PendingNotify {
    cseq: u32,
    branch: String,
    sent_at: u64,
    retry_count: u32,
}

/// Handles an incoming `SUBSCRIBE` request from the upstream platform.
pub(crate) fn handle_subscribe<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let response_tag = cascade.next_local_tag(now);

    let SipMessage::Request { headers, .. } = &msg else {
        return Vec::new();
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

    let Some(call_id_header) = headers.get(&HeaderName::CallId) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    };
    let call_id = call_id_header.as_str().trim().to_string();
    if call_id.is_empty() {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    }

    let Some(from) = headers.get(&HeaderName::From) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    };
    let Some(remote_tag) = extract_tag(from) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    };

    let Some(event_header) = headers.get(&HeaderName::Other("Event".to_string())) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    };
    let Some(event_package) = canonical_event_package(event_header.as_str().trim()) else {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            489,
            "Bad Event",
            &response_tag,
            Vec::new(),
        ))];
    };

    let remote_uri = headers
        .get(&HeaderName::Contact)
        .and_then(parse_uri_from_header)
        .or_else(|| parse_uri_from_header(from))
        .unwrap_or_else(|| cascade.config.upstream.clone());

    let requested_expiry = headers
        .get(&HeaderName::Expires)
        .and_then(|v| v.as_str().trim().parse::<u64>().ok())
        .unwrap_or(cascade.config.subscription_default_expiry_seconds as u64);

    let local_tag = headers
        .get(&HeaderName::To)
        .and_then(extract_tag)
        .unwrap_or_else(|| cascade.next_local_tag(now));

    let key = subscription_key(&call_id, &remote_tag);

    if requested_expiry == 0 {
        let final_notify = cascade.subscriptions.remove(&key).and_then(|sub| {
            let cseq = sub.next_cseq;
            let branch = cascade.next_branch(&sub.call_id, cseq);
            build_notify(
                cascade,
                &sub,
                cseq,
                &branch,
                "terminated;reason=timeout",
                now,
            )
            .ok()
        });
        let mut ok = build_ok_response(&msg, &local_tag);
        ok.headers_mut()
            .append(HeaderName::Expires, HeaderValue::new("0"));
        let mut outputs = vec![CascadeOutput::SendResponse(ok)];
        if let Some(n) = final_notify {
            outputs.push(CascadeOutput::SendRequest(n));
        }
        return outputs;
    }

    let granted_expiry = requested_expiry.clamp(
        cascade.config.subscription_min_expiry_seconds as u64,
        cascade.config.subscription_max_expiry_seconds as u64,
    );

    enforce_subscription_capacity(cascade, &key);

    let mut sub = cascade
        .subscriptions
        .remove(&key)
        .unwrap_or_else(|| Subscription {
            call_id: call_id.clone(),
            local_tag: local_tag.clone(),
            remote_tag: remote_tag.clone(),
            remote_uri: remote_uri.clone(),
            event_package: event_package.clone(),
            expires_at: 0,
            next_cseq: 1,
            pending_notify: None,
            last_active_at: 0,
        });
    sub.local_tag = local_tag.clone();
    sub.remote_uri = remote_uri;
    sub.event_package = event_package;
    sub.expires_at = now.saturating_add(granted_expiry);
    sub.pending_notify = None;
    sub.last_active_at = now;

    let notify = {
        let cseq = sub.next_cseq;
        sub.next_cseq = sub.next_cseq.saturating_add(1);
        let branch = cascade.next_branch(&sub.call_id, cseq);
        let remaining = sub.expires_at.saturating_sub(now);
        match build_notify(
            cascade,
            &sub,
            cseq,
            &branch,
            &format!("active;expires={remaining}"),
            now,
        ) {
            Ok(n) => {
                sub.pending_notify = Some(PendingNotify {
                    cseq,
                    branch,
                    sent_at: now,
                    retry_count: 0,
                });
                Some(n)
            }
            Err(e) => {
                tracing::warn!("failed to encode initial NOTIFY: {e}");
                let mut err_response =
                    build_response(&msg, 500, "Internal Server Error", &local_tag, Vec::new());
                err_response
                    .headers_mut()
                    .append(HeaderName::Expires, HeaderValue::new("0"));
                return vec![CascadeOutput::SendResponse(err_response)];
            }
        }
    };

    cascade.subscriptions.insert(key, sub);

    let mut ok = build_ok_response(&msg, &local_tag);
    ok.headers_mut().append(
        HeaderName::Expires,
        HeaderValue::new(granted_expiry.to_string()),
    );

    let mut outputs = vec![CascadeOutput::SendResponse(ok)];
    if let Some(n) = notify {
        outputs.push(CascadeOutput::SendRequest(n));
    }
    outputs
}

/// Processes a response to an outbound `NOTIFY` request.
pub(crate) fn handle_response<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    _now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let (cseq_num, call_id) = match &msg {
        SipMessage::Response { .. } => {
            let cseq = match msg.cseq() {
                Some(c) => c,
                None => return Vec::new(),
            };
            let call_id = match msg.call_id() {
                Some(c) => c.to_string(),
                None => return Vec::new(),
            };
            (cseq.0, call_id)
        }
        SipMessage::Request { .. } => return Vec::new(),
    };

    let key = cascade
        .subscriptions
        .iter()
        .find(|(_, s)| s.call_id == call_id)
        .map(|(k, _)| k.clone());
    if let Some(key) = key
        && let Some(sub) = cascade.subscriptions.get_mut(&key)
        && sub
            .pending_notify
            .as_ref()
            .is_some_and(|p| p.cseq == cseq_num)
    {
        sub.pending_notify = None;
    }
    Vec::new()
}

/// Processes timer expiry and pending `NOTIFY` retransmissions for all
/// subscriptions.
pub(crate) fn on_tick<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
) -> Result<Vec<CascadeOutput>, CascadeError> {
    let mut outputs = Vec::new();
    let keys: Vec<String> = cascade.subscriptions.keys().cloned().collect();

    for key in keys {
        let Some(mut sub) = cascade.subscriptions.remove(&key) else {
            continue;
        };

        let expired = now >= sub.expires_at;
        if expired {
            let cseq = sub.next_cseq;
            let branch = cascade.next_branch(&sub.call_id, cseq);
            if let Ok(notify) = build_notify(
                cascade,
                &sub,
                cseq,
                &branch,
                "terminated;reason=timeout",
                now,
            ) {
                outputs.push(CascadeOutput::SendRequest(notify));
            }
            continue;
        }

        if let Some(pending) = &sub.pending_notify
            && now.saturating_sub(pending.sent_at) >= NOTIFY_TIMEOUT_SECONDS
        {
            if pending.retry_count >= NOTIFY_MAX_RETRIES {
                let cseq = sub.next_cseq;
                let branch = cascade.next_branch(&sub.call_id, cseq);
                if let Ok(notify) = build_notify(
                    cascade,
                    &sub,
                    cseq,
                    &branch,
                    "terminated;reason=timeout",
                    now,
                ) {
                    outputs.push(CascadeOutput::SendRequest(notify));
                }
                continue;
            }

            let remaining = sub.expires_at.saturating_sub(now);
            let notify = build_notify(
                cascade,
                &sub,
                pending.cseq,
                &pending.branch,
                &format!("active;expires={remaining}"),
                now,
            )?;
            sub.pending_notify = Some(PendingNotify {
                cseq: pending.cseq,
                branch: pending.branch.clone(),
                sent_at: now,
                retry_count: pending.retry_count + 1,
            });
            outputs.push(CascadeOutput::SendRequest(notify));
        }

        cascade.subscriptions.insert(key, sub);
    }

    Ok(outputs)
}

fn subscription_key(call_id: &str, remote_tag: &str) -> String {
    format!("{call_id}:{remote_tag}")
}

fn extract_tag(header: &HeaderValue) -> Option<String> {
    header
        .as_str()
        .split(';')
        .find_map(|param| {
            let param = param.trim();
            param
                .strip_prefix("tag=")
                .map(|v| v.trim().trim_matches('"').to_string())
        })
        .filter(|t| !t.is_empty())
}

fn canonical_event_package(raw: &str) -> Option<String> {
    let base = raw.split(';').next().unwrap_or("").trim();
    let lower = base.to_ascii_lowercase();
    SUPPORTED_PACKAGES
        .iter()
        .find(|p| p.to_ascii_lowercase() == lower)
        .copied()
        .map(String::from)
}

fn enforce_subscription_capacity<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    new_key: &str,
) {
    if cascade.subscriptions.len() < cascade.config.subscription_max_subscriptions as usize
        || cascade.subscriptions.contains_key(new_key)
    {
        return;
    }
    let Some(oldest) = cascade
        .subscriptions
        .iter()
        .min_by_key(|(_, s)| s.last_active_at)
        .map(|(k, _)| k.clone())
    else {
        return;
    };
    cascade.subscriptions.remove(&oldest);
}

mod notify;
use notify::build_notify;
