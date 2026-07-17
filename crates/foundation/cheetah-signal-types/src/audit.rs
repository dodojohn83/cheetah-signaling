//! Audit logging types and port.
//!
//! Audit events capture security-relevant actions without recording secrets or
//! full protocol bodies. Implementations of `AuditLog` may write to storage,
//! stdout, or a message bus, but the trait itself is synchronous and
//! fire-and-forget.

use crate::{NodeId, TenantId, UtcTimestamp};

/// A security/operational audit event.
#[derive(Clone, Debug, PartialEq)]
pub struct AuditEvent {
    /// Monotonic wall timestamp in UTC.
    pub timestamp: UtcTimestamp,
    /// Action identifier, e.g. `auth.login`, `device.delete`, `tenant.update`.
    pub action: String,
    /// Actor that performed the action, e.g. a principal ID or service name.
    pub actor: String,
    /// Tenant scope when known.
    pub tenant_id: Option<TenantId>,
    /// Target type, e.g. `device`, `tenant`, `media_session`, `plugin`.
    pub target_type: String,
    /// Target identifier when available.
    pub target_id: Option<String>,
    /// Outcome of the action.
    pub outcome: AuditOutcome,
    /// Request identifier for correlation with logs and traces.
    pub request_id: String,
    /// Correlation identifier when propagated from the client.
    pub correlation_id: Option<String>,
    /// Source IP of the request when available.
    pub source_ip: Option<String>,
    /// Node that processed the request.
    pub node_id: NodeId,
    /// Optional safe details; must never contain secrets or raw protocol bodies.
    pub details: Option<String>,
}

/// Result of an audited action.
#[derive(Clone, Debug, PartialEq)]
pub enum AuditOutcome {
    /// The action completed successfully.
    Success,
    /// The action failed for a non-authorization reason.
    Failure {
        /// Stable reason code or short description.
        reason: String,
    },
    /// The action was denied due to authorization or policy.
    Denied {
        /// Stable reason code or short description.
        reason: String,
    },
}

/// Sink for audit events.
pub trait AuditLog: Send + Sync {
    /// Records an audit event.
    ///
    /// Implementations must not block the caller on durable writes; they should
    /// either write to a fast buffer, spawn a task, or drop events when back
    /// pressure would otherwise block the request path.
    fn record(&self, event: AuditEvent);
}

/// Audit sink that discards every event.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpAuditLog;

impl AuditLog for NoOpAuditLog {
    fn record(&self, _event: AuditEvent) {}
}
