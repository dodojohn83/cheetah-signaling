//! Extension repository ports that do not belong to the domain UnitOfWork.

use crate::StorageError;
use cheetah_domain::{ClusterNode, NodeLoad, OwnerInfo};
use cheetah_signal_types::{
    DeviceId, NodeId, NodeInstanceId, OperationId, Page, PageRequest, TenantId, UtcTimestamp,
};

/// Repository for device owner leases.
#[async_trait::async_trait]
pub trait OwnerRepository: Send + Sync {
    /// Gets the current owner for a device.
    async fn get(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<OwnerInfo>, StorageError>;

    /// Sets the owner for a device.
    async fn set(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        owner: OwnerInfo,
    ) -> Result<(), StorageError>;

    /// Clears the owner for a device.
    async fn clear(&mut self, tenant_id: TenantId, device_id: DeviceId)
    -> Result<(), StorageError>;

    /// Atomically acquires or re-acquires ownership of a device.
    ///
    /// If the device has no owner, the existing lease has expired, or the
    /// current owner's node is no longer alive, the `node_id` becomes the new
    /// owner and the epoch is incremented. Returns the new [`OwnerInfo`] on
    /// success.
    async fn acquire(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        now: UtcTimestamp,
        lease_until: UtcTimestamp,
    ) -> Result<OwnerInfo, StorageError>;

    /// Extends an existing lease if `node_id` still owns the device and the
    /// lease has not expired.
    async fn renew(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        lease_until: cheetah_signal_types::UtcTimestamp,
    ) -> Result<Option<OwnerInfo>, StorageError>;

    /// Releases ownership if `node_id` and `epoch` match the current record.
    async fn release(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        epoch: cheetah_signal_types::OwnerEpoch,
    ) -> Result<(), StorageError>;
}

/// A single dispatch attempt for an operation.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct OperationStep {
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Operation identifier.
    pub operation_id: OperationId,
    /// Attempt number.
    pub attempt: u32,
    /// Owner epoch at the time of the attempt.
    pub owner_epoch: u64,
    /// Status summary.
    pub status: String,
    /// Optional error message.
    pub error: Option<String>,
}

impl OperationStep {
    /// Creates a new operation step.
    pub fn new(
        tenant_id: TenantId,
        operation_id: OperationId,
        attempt: u32,
        owner_epoch: u64,
        status: impl Into<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            tenant_id,
            operation_id,
            attempt,
            owner_epoch,
            status: status.into(),
            error,
        }
    }
}

/// Repository for operation dispatch attempts.
#[async_trait::async_trait]
pub trait OperationStepRepository: Send + Sync {
    /// Records an operation step.
    async fn record(&mut self, step: OperationStep) -> Result<(), StorageError>;

    /// Lists steps for an operation ordered by attempt.
    async fn list(
        &mut self,
        tenant_id: TenantId,
        operation_id: OperationId,
    ) -> Result<Vec<OperationStep>, StorageError>;
}

/// Repository for cluster node registrations and leases.
#[async_trait::async_trait]
pub trait NodeRepository: Send + Sync {
    /// Registers or re-registers a node. A re-registration with a new
    /// `instance_id` overwrites the previous incarnation, fencing it.
    async fn register(&mut self, node: ClusterNode) -> Result<(), StorageError>;

    /// Extends the lease and updates load for `node_id`, but only if the
    /// current `instance_id` matches. Returns the updated node, or `None` if
    /// the node is unknown or has been fenced by another instance.
    async fn heartbeat(
        &mut self,
        node_id: NodeId,
        instance_id: NodeInstanceId,
        lease_until: UtcTimestamp,
        updated_at: UtcTimestamp,
        load: NodeLoad,
    ) -> Result<Option<ClusterNode>, StorageError>;

    /// Returns the registered node, if any.
    async fn get(&self, node_id: NodeId) -> Result<Option<ClusterNode>, StorageError>;

    /// Lists nodes whose lease is still valid at `now`, paginated by cursor.
    async fn list_alive(
        &self,
        now: UtcTimestamp,
        page: PageRequest,
    ) -> Result<Page<ClusterNode>, StorageError>;

    /// Marks the node as draining if `instance_id` matches.
    /// Returns `true` if the row was updated, or `false` if the node is
    /// unknown or has been fenced by another instance.
    async fn mark_draining(
        &mut self,
        node_id: NodeId,
        instance_id: NodeInstanceId,
        updated_at: UtcTimestamp,
    ) -> Result<bool, StorageError>;
}
