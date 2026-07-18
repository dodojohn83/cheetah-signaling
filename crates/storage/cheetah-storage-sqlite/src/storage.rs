//! SQLite storage implementation.

use crate::error::sqlx_to_storage;
use crate::migration::SqliteMigration;
use crate::operation_step::SqliteOperationStepRepository;
use crate::owner::{SqliteDeviceOwnerResolver, SqliteOwnerRepository};
use crate::unit_of_work::SqliteUnitOfWork;
use cheetah_domain::Clock;
use cheetah_domain::ports::DeviceOwnerResolver;
use cheetah_storage_api::{
    Migration as MigrationTrait, OperationStepRepository, OwnerRepository, Storage, StorageError,
};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// SQLite storage adapter.
#[derive(Debug)]
pub struct SqliteStorage {
    write_pool: SqlitePool,
    read_pool: SqlitePool,
    migration: SqliteMigration,
}

impl SqliteStorage {
    /// Creates a new SQLite storage at the given path.
    pub async fn new(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_millis(5000));

        let write_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(None)
            .idle_timeout(Duration::from_secs(60))
            .connect_with(options.clone())
            .await
            .map_err(sqlx_to_storage)?;

        let read_pool = SqlitePoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(None)
            .idle_timeout(Duration::from_secs(60))
            .connect_with(options)
            .await
            .map_err(sqlx_to_storage)?;

        let migration = SqliteMigration::new(write_pool.clone());

        Ok(Self {
            write_pool,
            read_pool,
            migration,
        })
    }

    /// Returns a device owner resolver backed by the read pool.
    pub fn device_owner_resolver(&self, clock: Box<dyn Clock>) -> Box<dyn DeviceOwnerResolver> {
        Box::new(SqliteDeviceOwnerResolver::new(
            clock,
            self.read_pool.clone(),
        ))
    }

    /// Connection pool used for writes.
    pub fn write_pool(&self) -> &SqlitePool {
        &self.write_pool
    }

    /// Connection pool used for reads.
    pub fn read_pool(&self) -> &SqlitePool {
        &self.read_pool
    }
}

#[async_trait::async_trait]
impl Storage for SqliteStorage {
    fn migration(&self) -> &dyn MigrationTrait {
        &self.migration
    }

    async fn begin(&self) -> Result<Box<dyn cheetah_domain::UnitOfWork>, StorageError> {
        let tx = self.write_pool.begin().await.map_err(sqlx_to_storage)?;
        Ok(Box::new(SqliteUnitOfWork::new(self.write_pool.clone(), tx)))
    }

    fn owner_repository(&self, clock: Arc<dyn Clock>) -> Box<dyn OwnerRepository> {
        Box::new(SqliteOwnerRepository::new(
            self.read_pool.clone(),
            self.write_pool.clone(),
            clock,
        ))
    }

    fn operation_step_repository(&self) -> Box<dyn OperationStepRepository> {
        Box::new(SqliteOperationStepRepository::new(self.write_pool.clone()))
    }

    async fn close(self) -> Result<(), StorageError> {
        self.write_pool.close().await;
        self.read_pool.close().await;
        Ok(())
    }
}
