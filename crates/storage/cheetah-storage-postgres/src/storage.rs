//! PostgreSQL storage implementation.

use crate::error::sqlx_to_storage;
use crate::media_node::PostgresMediaNodeRepository;
use crate::migration::PostgresMigration;
use crate::node::PostgresNodeRepository;
use crate::operation_step::PostgresOperationStepRepository;
use crate::owner::{PostgresDeviceOwnerResolver, PostgresOwnerRepository};
use crate::tenant::PostgresTenantRepository;
use crate::unit_of_work::PostgresUnitOfWork;
use cheetah_domain::Clock;
use cheetah_domain::ports::DeviceOwnerResolver;
use cheetah_storage_api::{
    MediaNodeRepository, Migration as MigrationTrait, NodeRepository, OperationStepRepository,
    OwnerRepository, Storage, StorageError, TenantRepository,
};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::time::Duration;

/// PostgreSQL storage adapter.
#[derive(Debug)]
pub struct PostgresStorage {
    write_pool: PgPool,
    read_pool: PgPool,
    migration: PostgresMigration,
}

impl PostgresStorage {
    /// Creates a new PostgreSQL storage from a connection string.
    pub async fn new(url: impl AsRef<str>) -> Result<Self, StorageError> {
        let url = url.as_ref();

        let options = PgConnectOptions::from_str(url)
            .map_err(|e| StorageError::config(e.to_string()))?
            .application_name("cheetah_signaling");

        let write_pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(Duration::from_secs(30 * 60))
            .idle_timeout(Duration::from_secs(60))
            .connect_with(options.clone())
            .await
            .map_err(sqlx_to_storage)?;

        let read_pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(Duration::from_secs(30 * 60))
            .idle_timeout(Duration::from_secs(60))
            .connect_with(options)
            .await
            .map_err(sqlx_to_storage)?;

        let migration = PostgresMigration::new(write_pool.clone());

        Ok(Self {
            write_pool,
            read_pool,
            migration,
        })
    }

    /// Returns a device owner resolver backed by the read pool.
    pub fn device_owner_resolver(&self, clock: Box<dyn Clock>) -> Box<dyn DeviceOwnerResolver> {
        Box::new(PostgresDeviceOwnerResolver::new(
            clock,
            self.read_pool.clone(),
        ))
    }

    /// Connection pool used for writes.
    pub fn write_pool(&self) -> &PgPool {
        &self.write_pool
    }

    /// Connection pool used for reads.
    pub fn read_pool(&self) -> &PgPool {
        &self.read_pool
    }
}

#[async_trait::async_trait]
impl Storage for PostgresStorage {
    fn migration(&self) -> &dyn MigrationTrait {
        &self.migration
    }

    async fn begin(&self) -> Result<Box<dyn cheetah_domain::UnitOfWork>, StorageError> {
        let tx = self.write_pool.begin().await.map_err(sqlx_to_storage)?;
        Ok(Box::new(PostgresUnitOfWork::new(
            self.write_pool.clone(),
            tx,
        )))
    }

    fn owner_repository(&self) -> Box<dyn OwnerRepository> {
        Box::new(PostgresOwnerRepository::new(
            self.read_pool.clone(),
            self.write_pool.clone(),
        ))
    }

    fn operation_step_repository(&self) -> Box<dyn OperationStepRepository> {
        Box::new(PostgresOperationStepRepository::new(
            self.write_pool.clone(),
        ))
    }

    fn node_repository(&self) -> Box<dyn NodeRepository> {
        Box::new(PostgresNodeRepository::new(
            self.read_pool.clone(),
            self.write_pool.clone(),
        ))
    }

    fn media_node_repository(&self) -> Box<dyn MediaNodeRepository> {
        Box::new(PostgresMediaNodeRepository::new(
            self.read_pool.clone(),
            self.write_pool.clone(),
        ))
    }

    fn tenant_repository(&self) -> Box<dyn TenantRepository> {
        Box::new(PostgresTenantRepository::new(
            self.read_pool.clone(),
            self.write_pool.clone(),
        ))
    }

    async fn close(self) -> Result<(), StorageError> {
        self.write_pool.close().await;
        self.read_pool.close().await;
        Ok(())
    }
}
