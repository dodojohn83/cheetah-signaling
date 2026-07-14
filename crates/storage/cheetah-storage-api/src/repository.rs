//! Extension repository ports that do not belong to the domain UnitOfWork.

use crate::StorageError;
use cheetah_domain::OwnerInfo;
use cheetah_signal_types::{DeviceId, OperationId, TenantId};

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
