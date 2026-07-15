//! Scheduler errors.

/// Errors returned by the media node registry and scheduler.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// No node matched the scheduling requirements.
    #[error("no media node available: {0}")]
    NoNode(String),
    /// The requested media node is not registered.
    #[error("media node not found: {0}")]
    NodeNotFound(String),
    /// The requested media node is at capacity.
    #[error("media node capacity exhausted: {0}")]
    CapacityExhausted(String),
    /// The reservation is not known to this scheduler.
    #[error("reservation not found: tenant={tenant_id}, binding={binding_id}")]
    ReservationNotFound {
        /// Tenant identifier.
        tenant_id: String,
        /// Media binding identifier.
        binding_id: String,
    },
    /// Invalid scheduling or registration input.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// The request was rejected because the TLS peer identity did not match.
    #[error("mTLS identity mismatch: expected {expected}, found {found:?}")]
    IdentityMismatch {
        /// Expected identity.
        expected: String,
        /// Found identity.
        found: Option<String>,
    },
}
