//! SQLite contract tests.

use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;
use cheetah_storage_tests::{contract::run_all, fixtures::Fixtures};

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_contract_suite() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let fixtures = Fixtures::new();
    let temp_file = std::env::temp_dir().join(format!(
        "cheetah-storage-tests-{}-{}.db",
        std::process::id(),
        fixtures.id_generator().generate_device_id().as_uuid()
    ));

    let _ = std::fs::remove_file(&temp_file);
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-wal"));
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-shm"));

    let storage = SqliteStorage::new(&temp_file).await?;
    run_all(&storage, &fixtures).await?;
    assert_negative_stored_revision_returns_internal(&storage, &fixtures).await?;
    storage.close().await?;

    let _ = std::fs::remove_file(&temp_file);
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-wal"));
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-shm"));

    Ok(())
}

async fn assert_negative_stored_revision_returns_internal(
    storage: &SqliteStorage,
    fixtures: &Fixtures,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let tenant_id = fixtures.tenant_id();
    let config = fixtures.webhook_config(tenant_id)?;

    let mut uow = storage.begin().await?;
    uow.webhook_config_repository().save(&config).await?;
    uow.commit().await?;

    // Simulate database corruption: set the stored revision to a negative value.
    let mut conn = storage.write_pool().acquire().await?;
    sqlx::query("UPDATE webhook_configs SET revision = -1 WHERE tenant_id = ? AND webhook_id = ?")
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
