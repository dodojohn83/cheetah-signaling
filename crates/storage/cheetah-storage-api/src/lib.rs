#![doc = include_str!("../README.md")]

//! Storage ports for Cheetah Signaling.

pub mod error;
pub mod migration;
pub mod phase_migration;
pub mod repository;
pub mod revision;
pub mod storage;
pub mod timestamp;

pub use revision::stored_revision_as_u64;

pub use error::StorageError;
pub use migration::{
    BackfillJob, BackfillProgress, Migration, MigrationInfo, MigrationPhase, MigrationStatus,
};
pub use phase_migration::{
    AppliedMigration, PhaseMigrationBackend, PhaseMigrationPlanner, PhaseMigrationRunner,
    VersionedMigration,
};
pub use repository::{
    MediaNodeRepository, NodeRepository, OperationStep, OperationStepRepository, OwnedDevice,
    OwnerRepository, TenantRepository,
};
pub use storage::Storage;
