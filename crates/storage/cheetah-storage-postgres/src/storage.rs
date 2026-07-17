//! PostgreSQL storage implementation.

use crate::error::sqlx_to_storage;
use crate::migration::PostgresMigration;
use crate::operation_step::PostgresOperationStepRepository;
use crate::owner::{PostgresDeviceOwnerResolver, PostgresOwnerRepository};
use crate::unit_of_work::PostgresUnitOfWork;
use cheetah_domain::Clock;
use cheetah_domain::ports::DeviceOwnerResolver;
use cheetah_storage_api::{
    Migration as MigrationTrait, OperationStepRepository, OwnerRepository, Storage, StorageError,
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
    migration_pool: PgPool,
    migration: PostgresMigration,
}

impl PostgresStorage {
    /// Creates a new PostgreSQL storage from a single connection string.
    ///
    /// The same connection string is used for both runtime and migration
    /// traffic. Prefer [`Self::new_with_roles`] for production deployments that
    /// separate the migration role from the runtime role.
    pub async fn new(url: impl AsRef<str>) -> Result<Self, StorageError> {
        let url = url.as_ref().to_string();
        Self::new_with_roles(url.clone(), url).await
    }

    /// Creates a new PostgreSQL storage with separate runtime and migration
    /// connection strings.
    ///
    /// The runtime pools are used for ordinary reads and writes, while the
    /// migration pool is used exclusively for schema migrations. This allows
    /// the runtime role to be granted only `SELECT`/`INSERT`/`UPDATE`/`DELETE`
    /// permissions, and the migration role to hold DDL privileges.
    pub async fn new_with_roles(
        runtime_url: impl AsRef<str>,
        migration_url: impl AsRef<str>,
    ) -> Result<Self, StorageError> {
        let runtime_url = runtime_url.as_ref();
        let migration_url = migration_url.as_ref();

        let runtime_options = PgConnectOptions::from_str(runtime_url)
            .map_err(|e| StorageError::config(e.to_string()))?
            .application_name("cheetah_signaling_runtime");

        let write_pool = PgPoolOptions::new()
            .max_connections(10)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(Duration::from_secs(30 * 60))
            .idle_timeout(Duration::from_secs(60))
            .connect_with(runtime_options.clone())
            .await
            .map_err(sqlx_to_storage)?;

        let read_pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(Duration::from_secs(30 * 60))
            .idle_timeout(Duration::from_secs(60))
            .connect_with(runtime_options)
            .await
            .map_err(sqlx_to_storage)?;

        let migration_options = PgConnectOptions::from_str(migration_url)
            .map_err(|e| StorageError::config(e.to_string()))?
            .application_name("cheetah_signaling_migrator");

        let migration_pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .max_lifetime(Duration::from_secs(30 * 60))
            .idle_timeout(Duration::from_secs(60))
            .connect_with(migration_options)
            .await
            .map_err(sqlx_to_storage)?;

        let migration = PostgresMigration::new(migration_pool.clone());

        Ok(Self {
            write_pool,
            read_pool,
            migration_pool,
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
        Ok(Box::new(PostgresUnitOfWork::new(tx)))
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

    async fn close(self) -> Result<(), StorageError> {
        self.write_pool.close().await;
        self.read_pool.close().await;
        self.migration_pool.close().await;
        Ok(())
    }
}
