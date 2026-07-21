//! HTTP API audit sink backed by `tracing`.

use crate::{ApiRequestContext, ApiState};
use cheetah_signal_types::{AuditEvent, AuditLog, AuditOutcome, SafeDetails};

/// Audit sink that emits each event as a structured `tracing::info!` log.
///
/// This is intentionally separate from request tracing: it records only
/// security-relevant actions and never records secrets or raw request bodies.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracingAuditLog;

impl AuditLog for TracingAuditLog {
    fn record(&self, event: AuditEvent) {
        let outcome = match &event.outcome {
            AuditOutcome::Success => "success",
            AuditOutcome::Failure { .. } => "failure",
            AuditOutcome::Denied { .. } => "denied",
        };
        let reason = match &event.outcome {
            AuditOutcome::Success => None,
            AuditOutcome::Failure { reason } | AuditOutcome::Denied { reason } => {
                Some(reason.as_str())
            }
        };

        tracing::info!(
            audit.action = %event.action,
            audit.actor = %event.actor,
            audit.tenant_id = ?event.tenant_id,
            audit.target_type = %event.target_type,
            audit.target_id = ?event.target_id,
            audit.outcome = %outcome,
            audit.reason = ?reason,
            audit.request_id = %event.request_id,
            audit.correlation_id = ?event.correlation_id,
            audit.source_ip = ?event.source_ip,
            audit.node_id = %event.node_id,
            audit.details = ?event.details,
            "audit event",
        );
    }
}

/// Records an audit event for a request.
///
/// `details` is redacted before being stored; callers do not need to pre-filter.
pub fn record(
    state: &ApiState,
    ctx: &ApiRequestContext,
    action: impl Into<String>,
    target_type: impl Into<String>,
    target_id: Option<String>,
    details: Option<String>,
    outcome: AuditOutcome,
) {
    let event = AuditEvent {
        timestamp: state.clock.now_wall(),
        action: action.into(),
        actor: ctx.principal.id.clone(),
        tenant_id: Some(ctx.tenant_id),
        target_type: target_type.into(),
        target_id,
        outcome,
        request_id: ctx.message_id.to_string(),
        correlation_id: Some(ctx.correlation_id.to_string()),
        source_ip: ctx.source_ip.clone(),
        node_id: ctx.node_id.unwrap_or(state.config.node_id),
        details: details.map(SafeDetails::new),
    };
    state.audit.record(event);
}
