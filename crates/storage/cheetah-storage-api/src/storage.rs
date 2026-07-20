//! Storage factory port.

use crate::{
    MediaNodeRepository, Migration, NodeRepository, OperationStepRepository, OwnerRepository,
    StorageError,
};
use cheetah_domain::UnitOfWork;

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
    fn owner_repository(&self) -> Box<dyn OwnerRepository>;

    /// Returns the operation step repository backed by the storage pools.
    fn operation_step_repository(&self) -> Box<dyn OperationStepRepository>;

    /// Returns the cluster node repository backed by the storage pools.
    fn node_repository(&self) -> Box<dyn NodeRepository>;

    /// Returns the media node repository backed by the storage pools.
    fn media_node_repository(&self) -> Box<dyn MediaNodeRepository>;

    /// Gracefully closes the storage.
    async fn close(self) -> Result<(), StorageError>;
}
