//! PostgreSQL contract tests.

use cheetah_storage_api::Storage;
use cheetah_storage_postgres::PostgresStorage;
use cheetah_storage_tests::{contract::run_all, fixtures::Fixtures};
use testcontainers_modules::{postgres, testcontainers::runners::AsyncRunner};

#[tokio::test(flavor = "multi_thread")]
async fn postgres_contract_suite() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let fixtures = Fixtures::new();
    let container = postgres::Postgres::default().start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(5432).await?;

    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres?sslmode=disable");
    let storage = PostgresStorage::new(url).await?;
    run_all(&storage, &fixtures).await?;
    storage.close().await?;

    Ok(())
}
