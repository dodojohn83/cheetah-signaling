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
    storage.close().await?;

    let _ = std::fs::remove_file(&temp_file);
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-wal"));
    let _ = std::fs::remove_file(temp_file.as_path().with_extension("db-shm"));

    Ok(())
}
