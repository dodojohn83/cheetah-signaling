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
use super::{CascadeCredentialProvider, CascadeOutput, Gb28181Cascade, State, validate_token};

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

#[cfg(test)]
impl Subscription {
    pub(crate) fn local_tag(&self) -> &str {
        &self.local_tag
    }
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
    if call_id.is_empty() || validate_token(&call_id).is_err() {
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
    let remote_tag = extract_tag(from);
    if remote_tag
        .as_ref()
        .is_some_and(|t| validate_token(t).is_err())
    {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    }
    let remote_tag = match remote_tag {
        Some(t) => t,
        None => {
            return vec![CascadeOutput::SendResponse(build_response(
                &msg,
                400,
                "Bad Request",
                &response_tag,
                Vec::new(),
            ))];
        }
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

    let requested_expiry = match headers.get(&HeaderName::Expires) {
        Some(value) => {
            let trimmed = value.as_str().trim();
            if trimmed.is_empty() {
                return vec![CascadeOutput::SendResponse(build_response(
                    &msg,
                    400,
                    "Bad Request",
                    &response_tag,
                    Vec::new(),
                ))];
            }
            match trimmed.parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    return vec![CascadeOutput::SendResponse(build_response(
                        &msg,
                        400,
                        "Bad Request",
                        &response_tag,
                        Vec::new(),
                    ))];
                }
            }
        }
        None => cascade.config.subscription_default_expiry_seconds as u64,
    };

    let key = subscription_key(&call_id, &remote_tag);
    let to_tag = headers.get(&HeaderName::To).and_then(extract_tag);
    if to_tag.as_ref().is_some_and(|t| validate_token(t).is_err()) {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            400,
            "Bad Request",
            &response_tag,
            Vec::new(),
        ))];
    }
    let local_tag = if let Some(sub) = cascade.subscriptions.get(&key) {
        to_tag.unwrap_or_else(|| sub.local_tag.clone())
    } else {
        to_tag.unwrap_or_else(|| cascade.next_local_tag(now))
    };

    let min_expiry = cascade.config.subscription_min_expiry_seconds as u64;
    let max_expiry = cascade.config.subscription_max_expiry_seconds as u64;
    let granted_expiry =
        requested_expiry.clamp(min_expiry.min(max_expiry), min_expiry.max(max_expiry));

    if let Some(existing) = cascade.subscriptions.get(&key)
        && existing.event_package != event_package
    {
        return vec![CascadeOutput::SendResponse(build_response(
            &msg,
            489,
            "Bad Event",
            &local_tag,
            Vec::new(),
        ))];
    }

    if requested_expiry == 0 {
        let final_notify = if let Some(sub) = cascade.subscriptions.get(&key).cloned() {
            let cseq = sub.next_cseq;
            let branch = cascade.next_branch(&sub.call_id, cseq);
            match build_notify(
                cascade,
                &sub,
                cseq,
                &branch,
                "terminated;reason=timeout",
                now,
            ) {
                Ok(n) => Some(n),
                Err(e) => {
                    tracing::warn!("failed to build final NOTIFY for subscription {key}: {e}");
                    None
                }
            }
        } else {
            None
        };
        cascade.subscriptions.remove(&key);
        let mut ok = build_ok_response(&msg, &local_tag);
        ok.headers_mut()
            .append(HeaderName::Expires, HeaderValue::new("0"));
        let mut outputs = vec![CascadeOutput::SendResponse(ok)];
        if let Some(n) = final_notify {
            outputs.push(CascadeOutput::SendRequest(n));
        }
        return outputs;
    }

    let next_cseq = cascade
        .subscriptions
        .get(&key)
        .map(|s| s.next_cseq)
        .unwrap_or(1);
    let branch = cascade.next_branch(&call_id, next_cseq);
    let expires_at = now.saturating_add(granted_expiry);
    let temp_sub = Subscription {
        call_id: call_id.clone(),
        local_tag: local_tag.clone(),
        remote_tag: remote_tag.clone(),
        remote_uri: remote_uri.clone(),
        event_package: event_package.clone(),
        expires_at,
        next_cseq,
        pending_notify: None,
        last_active_at: now,
    };

    let notify = match build_notify(
        &*cascade,
        &temp_sub,
        next_cseq,
        &branch,
        &format!("active;expires={granted_expiry}"),
        now,
    ) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!("failed to build initial NOTIFY for subscription {key}: {e}");
            let mut err_response =
                build_response(&msg, 500, "Internal Server Error", &local_tag, Vec::new());
            err_response
                .headers_mut()
                .append(HeaderName::Expires, HeaderValue::new("0"));
            return vec![CascadeOutput::SendResponse(err_response)];
        }
    };

    let mut outputs = Vec::new();
    if let Some(evicted) = enforce_subscription_capacity(cascade, &key, now) {
        outputs.push(evicted);
    }

    let mut sub = temp_sub;
    sub.next_cseq = next_cseq.saturating_add(1);
    sub.pending_notify = Some(PendingNotify {
        cseq: next_cseq,
        branch,
        sent_at: now,
        retry_count: 0,
    });
    cascade.subscriptions.remove(&key);
    cascade.subscriptions.insert(key, sub);

    let mut ok = build_ok_response(&msg, &local_tag);
    ok.headers_mut().append(
        HeaderName::Expires,
        HeaderValue::new(granted_expiry.to_string()),
    );

    outputs.push(CascadeOutput::SendResponse(ok));
    outputs.push(CascadeOutput::SendRequest(notify));
    outputs
}

/// Processes a response to an outbound `NOTIFY` request.
pub(crate) fn handle_response<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    _now: u64,
    msg: SipMessage,
) -> Vec<CascadeOutput> {
    let (status_code, cseq_num, call_id) = match &msg {
        SipMessage::Response { line, .. } => {
            let cseq = match msg.cseq() {
                Ok(c) => c,
                Err(_) => return Vec::new(),
            };
            let call_id = match msg.call_id() {
                Some(c) => c.to_string(),
                None => return Vec::new(),
            };
            (line.code, cseq.0, call_id)
        }
        SipMessage::Request { .. } => return Vec::new(),
    };

    let key = cascade
        .subscriptions
        .iter()
        .find(|(_, s)| s.call_id == call_id)
        .map(|(k, _)| k.clone());
    let Some(key) = key else {
        return Vec::new();
    };

    // Only act on final responses to a pending NOTIFY. 2xx stops retransmission;
    // any final error (3xx/4xx/5xx/6xx) terminates the subscription because the
    // upstream has rejected the dialog usage.
    if (200..300).contains(&status_code) {
        if let Some(sub) = cascade.subscriptions.get_mut(&key)
            && sub
                .pending_notify
                .as_ref()
                .is_some_and(|p| p.cseq == cseq_num)
        {
            sub.pending_notify = None;
        }
    } else if status_code >= 300 {
        cascade.subscriptions.remove(&key);
    }
    Vec::new()
}

/// Processes timer expiry and pending `NOTIFY` retransmissions for all
/// subscriptions.
pub(crate) fn on_tick<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
) -> Vec<CascadeOutput> {
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
            match build_notify(
                cascade,
                &sub,
                cseq,
                &branch,
                "terminated;reason=timeout",
                now,
            ) {
                Ok(notify) => outputs.push(CascadeOutput::SendRequest(notify)),
                Err(e) => tracing::warn!("failed to build expiry NOTIFY for {key}: {e}"),
            }
            continue;
        }

        if let Some(pending) = &sub.pending_notify
            && now.saturating_sub(pending.sent_at) >= NOTIFY_TIMEOUT_SECONDS
        {
            if pending.retry_count >= NOTIFY_MAX_RETRIES {
                let cseq = sub.next_cseq;
                let branch = cascade.next_branch(&sub.call_id, cseq);
                match build_notify(
                    cascade,
                    &sub,
                    cseq,
                    &branch,
                    "terminated;reason=timeout",
                    now,
                ) {
                    Ok(notify) => outputs.push(CascadeOutput::SendRequest(notify)),
                    Err(e) => tracing::warn!("failed to build timeout NOTIFY for {key}: {e}"),
                }
                continue;
            }

            let remaining = sub.expires_at.saturating_sub(now);
            match build_notify(
                cascade,
                &sub,
                pending.cseq,
                &pending.branch,
                &format!("active;expires={remaining}"),
                now,
            ) {
                Ok(notify) => {
                    sub.pending_notify = Some(PendingNotify {
                        cseq: pending.cseq,
                        branch: pending.branch.clone(),
                        sent_at: now,
                        retry_count: pending.retry_count + 1,
                    });
                    outputs.push(CascadeOutput::SendRequest(notify));
                }
                Err(e) => {
                    tracing::warn!("failed to build retransmission NOTIFY: {e}");
                    sub.pending_notify = None;
                }
            }
        }

        cascade.subscriptions.insert(key, sub);
    }

    outputs
}

fn subscription_key(call_id: &str, remote_tag: &str) -> String {
    format!("{call_id}:{remote_tag}")
}

fn extract_tag(header: &HeaderValue) -> Option<String> {
    cheetah_gb28181_core::sip::dialog::extract_tag(header.as_str()).map(str::to_string)
}

fn canonical_event_package(raw: &str) -> Option<String> {
    let base = raw.split(';').next().unwrap_or("").trim();
    SUPPORTED_PACKAGES
        .iter()
        .find(|p| p.eq_ignore_ascii_case(base))
        .copied()
        .map(String::from)
}

fn enforce_subscription_capacity<P: CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    new_key: &str,
    now: u64,
) -> Option<CascadeOutput> {
    if cascade.subscriptions.len() < cascade.config.subscription_max_subscriptions as usize
        || cascade.subscriptions.contains_key(new_key)
    {
        return None;
    }
    let oldest = cascade
        .subscriptions
        .iter()
        .min_by_key(|(_, s)| s.last_active_at)
        .map(|(k, _)| k.clone())?;
    let sub = cascade.subscriptions.get(&oldest)?.clone();
    let cseq = sub.next_cseq;
    let branch = cascade.next_branch(&sub.call_id, cseq);
    match build_notify(
        cascade,
        &sub,
        cseq,
        &branch,
        "terminated;reason=timeout",
        now,
    ) {
        Ok(notify) => {
            cascade.subscriptions.remove(&oldest);
            Some(CascadeOutput::SendRequest(notify))
        }
        Err(e) => {
            tracing::warn!("failed to build capacity-eviction NOTIFY for {oldest}: {e}");
            None
        }
    }
}

mod notify;
use notify::build_notify;
