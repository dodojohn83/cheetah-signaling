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
    assert_negative_stored_revision_returns_internal(&storage, &fixtures).await?;

    storage.close().await?;
    Ok(())
}

async fn assert_negative_stored_revision_returns_internal(
    storage: &PostgresStorage,
    fixtures: &Fixtures,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id = fixtures.tenant_id();
    let config = fixtures.webhook_config(tenant_id)?;

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&config).await?;
    uow.commit().await?;

    // Simulate database corruption: set the stored revision to a negative value.
    let mut conn = storage.write_pool().acquire().await?;
    sqlx::query(
        "UPDATE webhook_configs SET revision = -1 WHERE tenant_id = $1 AND webhook_id = $2",
    )
    .bind(tenant_id.as_uuid())
    .bind(config.webhook_id().as_uuid())
    .execute(&mut *conn)
    .await?;
    drop(conn);

    let mut stale = config.clone();
    stale.update(
        fixtures.clock(),
        Some("https://example.com/updated".to_string()),
        None,
        None,
        None,
    )?;

    let mut uow = storage.begin().await?;
    let result = uow.webhook_config_repository().save(&stale).await;
    assert!(
        matches!(result, Err(cheetah_domain::DomainError::Internal { .. })),
        "expected Internal for negative stored revision, got {:?}",
        result
    );
    uow.rollback().await?;

    Ok(())
}
