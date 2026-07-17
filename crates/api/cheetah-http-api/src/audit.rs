//! HTTP API audit sink backed by `tracing`.

use cheetah_signal_types::{AuditEvent, AuditLog, AuditOutcome};

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
