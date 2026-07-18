//! Storage factory port.

use crate::{Migration, OperationStepRepository, OwnerRepository, StorageError};
use cheetah_domain::{Clock, UnitOfWork};
use std::sync::Arc;

/// Storage abstraction.
///
/// A storage implementation is responsible for connection lifecycle, schema
/// migrations, and producing [`UnitOfWork`] instances.
#[async_trait::async_trait]
pub trait Storage: Send + Sync {
    /// Returns the migration runner for this storage.
    fn migration(&self) -> &dyn Migration;

    /// Begins a new unit of work.
    async fn begin(&self) -> Result<Box<dyn UnitOfWork>, StorageError>;

    /// Returns the device owner repository backed by the storage pools.
    fn owner_repository(&self, clock: Arc<dyn Clock>) -> Box<dyn OwnerRepository>;

    /// Returns the operation step repository backed by the storage pools.
    fn operation_step_repository(&self) -> Box<dyn OperationStepRepository>;

    /// Gracefully closes the storage.
    async fn close(self) -> Result<(), StorageError>;
}
