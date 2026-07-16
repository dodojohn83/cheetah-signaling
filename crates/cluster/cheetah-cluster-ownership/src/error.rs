//! Errors returned by cluster ownership operations.

use cheetah_signal_types::SignalError;
use cheetah_storage_api::StorageError;

/// Errors returned by device assignment operations.
#[derive(Debug, thiserror::Error)]
pub enum DeviceAssignmentError {
    /// No alive node can satisfy the assignment constraints.
    #[error("no available node for device")]
    NoAvailableNode,
    /// The assignment rate limit was exceeded.
    #[error("assignment rate limited")]
    RateLimited,
    /// Storage backend error.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// Domain invariant violation.
    #[error("domain error: {0}")]
    Domain(#[from] cheetah_domain::DomainError),
    /// Invalid argument (e.g., bad pagination).
    #[error("{0}")]
    InvalidArgument(#[from] SignalError),
}

/// Errors returned by rolling-upgrade and drain operations.
#[derive(Debug, thiserror::Error)]
pub enum RollingUpgradeError {
    /// The target node does not exist.
    #[error("node {0} not found")]
    NodeNotFound(String),
    /// The target node is not draining, so devices cannot be migrated off it.
    #[error("node {0} is not draining")]
    NotDraining(String),
    /// The target node has been fenced by a newer instance.
    #[error("node {0} has been fenced by a new instance")]
    Fenced(String),
    /// Assignment service error.
    #[error("assignment error: {0}")]
    Assignment(#[from] DeviceAssignmentError),
    /// Storage backend error.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// Device lookup failed.
    #[error("device lookup error: {0}")]
    Lookup(String),
    /// Invalid argument (e.g., bad pagination).
    #[error("{0}")]
    InvalidArgument(#[from] SignalError),
}
