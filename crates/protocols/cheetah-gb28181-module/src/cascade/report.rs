//! Upstream event reporting queue for a GB28181 cascade platform.
//!
//! The cascade receives domain events (presence changes, alarms, mobile
//! positions) and, when registered with an upstream platform, forwards them as
//! SIP `MESSAGE` requests carrying MANSCDP XML payloads. State-style events
//! (presence, mobile position) are merged per device so that only the latest
//! snapshot is sent. Alarms are preserved individually and carry an
//! idempotency key so the application layer can deduplicate them through an
//! outbox if needed.

use cheetah_gb28181_core::{
    Body, HeaderName, HeaderValue, Method, RequestLine, SipHeaders, SipMessage,
};

use super::{CascadeError, Gb28181Cascade, State, validate_token};
use crate::events::{DevicePresence, Gb28181Event};
use crate::xml::{build_alarm_notify, build_device_status_notify, build_mobile_position_notify};

/// Number of seconds after which an unflushed state snapshot is discarded.
/// This prevents the merge map from holding stale state forever while the
/// cascade is not registered.
const STATE_REPORT_TTL_SECONDS: u64 = 300;

/// Classification of a reportable upstream event. Used as part of the merge
/// key for state-style reports.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum ReportKind {
    Presence,
    MobilePosition,
    Alarm,
}

/// A pending upstream event report.
#[derive(Clone, Debug)]
pub(crate) struct PendingReport {
    kind: ReportKind,
    device_id: String,
    body_xml: String,
    is_critical: bool,
    idempotency_key: Option<String>,
    expires_at: u64,
}

impl PendingReport {
    fn merge_key(&self) -> String {
        format!(
            "{:?}:{}:{}",
            self.kind,
            self.device_id,
            self.idempotency_key.as_deref().unwrap_or("")
        )
    }
}

/// Enqueue a domain event for upstream reporting, converting it to a MANSCDP
/// NOTIFY payload when applicable.
pub(crate) fn enqueue<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    event: Gb28181Event,
) -> Result<Vec<super::CascadeOutput>, CascadeError> {
    let report = match event {
        Gb28181Event::DevicePresenceChanged {
            device_id,
            presence,
            ..
        } => {
            let device_id = device_id.to_string();
            let sn = next_sn(cascade);
            let body =
                build_device_status_notify(&sn, &device_id, presence == DevicePresence::Online)
                    .map_err(|e| {
                        CascadeError::Internal(format!("failed to encode DeviceStatus XML: {e}"))
                    })?;
            PendingReport {
                kind: ReportKind::Presence,
                device_id,
                body_xml: body,
                is_critical: false,
                idempotency_key: None,
                expires_at: now.saturating_add(STATE_REPORT_TTL_SECONDS),
            }
        }
        Gb28181Event::AlarmReceived {
            device_id,
            sn,
            priority,
            method,
            alarm_type,
            time,
            info,
            ..
        } => {
            let device_id = device_id.to_string();
            let idempotency_key = Some(format!(
                "{}-{}-{}-{}",
                device_id,
                alarm_type.as_deref().unwrap_or(""),
                time.as_deref().unwrap_or(""),
                sn
            ));
            let body = build_alarm_notify(
                &sn,
                &device_id,
                priority.as_deref(),
                method.as_deref(),
                alarm_type.as_deref(),
                time.as_deref(),
                info.as_deref(),
            )
            .map_err(|e| CascadeError::Internal(format!("failed to encode Alarm XML: {e}")))?;
            PendingReport {
                kind: ReportKind::Alarm,
                device_id,
                body_xml: body,
                is_critical: true,
                idempotency_key,
                expires_at: now.saturating_add(STATE_REPORT_TTL_SECONDS),
            }
        }
        Gb28181Event::MobilePositionReceived {
            device_id,
            time,
            longitude,
            latitude,
            speed,
            direction,
            altitude,
            ..
        } => {
            let device_id = device_id.to_string();
            let sn = next_sn(cascade);
            let body = build_mobile_position_notify(
                &sn,
                &device_id,
                time.as_deref(),
                longitude.as_deref(),
                latitude.as_deref(),
                speed.as_deref(),
                direction.as_deref(),
                altitude.as_deref(),
            )
            .map_err(|e| {
                CascadeError::Internal(format!("failed to encode MobilePosition XML: {e}"))
            })?;
            PendingReport {
                kind: ReportKind::MobilePosition,
                device_id,
                body_xml: body,
                is_critical: false,
                idempotency_key: None,
                expires_at: now.saturating_add(STATE_REPORT_TTL_SECONDS),
            }
        }
        _ => return Ok(Vec::new()),
    };

    enqueue_report(cascade, report);
    Ok(Vec::new())
}

/// Flush all pending reports into SIP `MESSAGE` requests when registered with
/// the upstream platform.
pub(crate) fn flush<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
) -> Result<Vec<super::CascadeOutput>, CascadeError> {
    prune_expired(cascade, now);
    if !matches!(cascade.state, State::Registered(_)) {
        return Ok(Vec::new());
    }

    // Move merged state snapshots into the flush queue one at a time. If the
    // queue is full, the snapshot stays in the merge map so the latest state is
    // not lost until capacity becomes available.
    let mut retained_order = std::collections::VecDeque::new();
    while let Some(key) = cascade.report_state_order.pop_front() {
        if let Some(report) = cascade.report_state.remove(&key) {
            if now < report.expires_at && push_to_queue(cascade, &report) {
                continue;
            }
            // Re-insert and remember the key; the entry was either expired or
            // could not be queued yet.
            cascade.report_state.insert(key.clone(), report);
            retained_order.push_back(key);
        }
    }
    cascade.report_state_order = retained_order;
    enforce_bounds(cascade);

    let mut outputs = Vec::new();
    while let Some(report) = cascade.report_queue.pop_front() {
        let request = build_message_request(cascade, now, &report)?;
        outputs.push(super::CascadeOutput::SendRequest(request));
    }
    Ok(outputs)
}

fn enqueue_report<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    report: PendingReport,
) {
    match report.kind {
        ReportKind::Presence | ReportKind::MobilePosition => {
            // State-style reports overwrite the previous snapshot for the same
            // device and event kind. Keep an LRU-style order list so we can
            // evict the oldest snapshot when the merge map is bounded.
            let key = report.merge_key();
            if cascade.report_state.insert(key.clone(), report).is_some()
                && let Some(pos) = cascade.report_state_order.iter().position(|k| k == &key)
            {
                let mut rest = cascade.report_state_order.split_off(pos + 1);
                cascade.report_state_order.pop_back();
                cascade.report_state_order.append(&mut rest);
            }
            cascade.report_state_order.push_back(key);
        }
        ReportKind::Alarm => {
            let _ = push_to_queue(cascade, &report);
        }
    }
    enforce_bounds(cascade);
}

/// Attempts to append `report` to the flush queue, evicting the oldest
/// non-critical entries to make room for a critical alarm. Returns `true` if
/// the report was enqueued.
fn push_to_queue<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    report: &PendingReport,
) -> bool {
    let max = cascade.config.report_max_queue_size as usize;
    if report.is_critical {
        // Drop oldest non-critical items to make room for a critical alarm.
        while cascade.report_queue.len() >= max
            && !cascade.report_queue.is_empty()
            && !cascade.report_queue.front().is_some_and(|r| r.is_critical)
        {
            cascade.report_queue.pop_front();
        }
    }
    if cascade.report_queue.len() < max {
        cascade.report_queue.push_back(report.clone());
        true
    } else if report.is_critical {
        // Queue is full of critical alarms; drop the oldest one.
        cascade.report_queue.pop_front();
        cascade.report_queue.push_back(report.clone());
        true
    } else {
        false
    }
}

fn enforce_bounds<P: super::CascadeCredentialProvider>(cascade: &mut Gb28181Cascade<P>) {
    let max = cascade.config.report_max_queue_size as usize;
    // The configured limit bounds the total of queued reports plus merged state
    // snapshots. Evict the oldest state snapshots first to preserve critical
    // alarms already in the queue.
    while cascade.report_state.len() + cascade.report_queue.len() > max
        && !cascade.report_state.is_empty()
    {
        if let Some(key) = cascade.report_state_order.pop_front() {
            cascade.report_state.remove(&key);
        } else {
            break;
        }
    }
    while cascade.report_queue.len() > max {
        cascade.report_queue.pop_front();
    }
}

fn prune_expired<P: super::CascadeCredentialProvider>(cascade: &mut Gb28181Cascade<P>, now: u64) {
    cascade.report_state.retain(|_, r| now < r.expires_at);
}

fn next_sn<P: super::CascadeCredentialProvider>(cascade: &mut Gb28181Cascade<P>) -> String {
    cascade.report_counter += 1;
    cascade.report_counter.to_string()
}

fn build_message_request<P: super::CascadeCredentialProvider>(
    cascade: &mut Gb28181Cascade<P>,
    now: u64,
    report: &PendingReport,
) -> Result<SipMessage, CascadeError> {
    let call_id = cascade.next_local_tag(now);
    let local_tag = call_id.clone();
    validate_token(&call_id)?;
    validate_token(&local_tag)?;

    let cseq = cascade.next_cseq();
    let branch = cascade.next_branch(&call_id, cseq);
    validate_token(&branch)?;

    let body: Body = report.body_xml.as_bytes().to_vec();
    let local_host = cascade.config.local_uri.host();
    let local_port = cascade.config.local_uri.port().unwrap_or(5060);

    let mut headers = SipHeaders::new();
    headers.append(
        HeaderName::Via,
        HeaderValue::via("UDP", local_host, local_port, &branch)?,
    );
    headers.append(
        HeaderName::From,
        HeaderValue::from_uri(&cascade.config.local_uri, &local_tag)?,
    );
    headers.append(
        HeaderName::To,
        HeaderValue::to_uri(&cascade.config.upstream),
    );
    headers.append(HeaderName::CallId, HeaderValue::new(call_id));
    headers.append(HeaderName::CSeq, HeaderValue::cseq(cseq, Method::Message));
    headers.append(
        HeaderName::ContentType,
        HeaderValue::new("Application/MANSCDP+xml"),
    );
    headers.append(HeaderName::MaxForwards, HeaderValue::new("70"));
    if let Some(ua) = &cascade.config.user_agent {
        headers.append(HeaderName::UserAgent, HeaderValue::new(ua.clone()));
    }
    if let Some(ref key) = report.idempotency_key {
        headers.append(
            HeaderName::Other("X-Idempotency-Key".to_string()),
            HeaderValue::new(key.clone()),
        );
    }
    headers.append(
        HeaderName::ContentLength,
        HeaderValue::new(body.len().to_string()),
    );

    Ok(SipMessage::Request {
        line: RequestLine::new(Method::Message, cascade.config.upstream.clone()),
        headers,
        body,
    })
}
