//! Errors returned by the cluster registry.

use cheetah_storage_api::StorageError;

/// Errors returned by cluster node lease operations.
#[derive(Debug, thiserror::Error)]
pub enum NodeLeaseError {
    /// Storage backend error.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// Domain invariant violation.
    #[error("domain error: {0}")]
    Domain(#[from] cheetah_domain::DomainError),
    /// The node has not been registered.
    #[error("node {0} is not registered")]
    NotRegistered(String),
    /// The node's lease has been taken over by a newer instance.
    #[error("node {0} has been fenced by a new instance")]
    Fenced(String),
    /// The node's version or contract versions are incompatible with the
    /// cluster's rolling-upgrade matrix.
    #[error("node is incompatible: {0}")]
    Incompatible(#[from] crate::compatibility::CompatibilityError),
}
