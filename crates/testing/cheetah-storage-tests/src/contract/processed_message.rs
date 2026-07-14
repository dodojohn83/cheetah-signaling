//! Idempotent processed message repository contract tests.

use crate::{contract::TestResult, fixtures::Fixtures};
use cheetah_domain::{ProcessedMessageRecord, ProcessedMessageStatus};
use cheetah_storage_api::Storage;

pub async fn run(storage: &dyn Storage, fixtures: &Fixtures) -> TestResult<()> {
    let tenant_id = fixtures.tenant_id();
    let message_id = fixtures.id_generator().generate_message_id();
    let now = fixtures.clock().now_wall();

    let mut uow = storage.begin().await?;

    let record = ProcessedMessageRecord {
        tenant_id,
        message_id,
        idempotency_key: Some("idempotent-key".to_string()),
        status: ProcessedMessageStatus::Pending,
        result_payload: None,
        processed_at: now,
        expires_at: None,
    };

    let inserted = uow
        .processed_message_repository()
        .get_or_insert(record.clone())
        .await?;
    assert!(inserted.is_none(), "first insert must succeed");

    let duplicate = uow
        .processed_message_repository()
        .get_or_insert(record.clone())
        .await?;
    assert!(duplicate.is_some(), "duplicate insert must return existing");
    let existing = duplicate.unwrap_or_else(|| unreachable!("duplicate must exist"));
    assert_eq!(existing.status, ProcessedMessageStatus::Pending);

    uow.processed_message_repository()
        .complete(
            tenant_id,
            message_id,
            ProcessedMessageStatus::Completed,
            Some(r#"{"ok":true}"#.to_string()),
            now,
        )
        .await?;
    uow.commit().await?;

    let mut uow = storage.begin().await?;
    let loaded = uow
        .processed_message_repository()
        .find(tenant_id, message_id)
        .await?;
    let loaded = loaded.unwrap_or_else(|| unreachable!("record must exist after commit"));
    assert_eq!(loaded.status, ProcessedMessageStatus::Completed);
    assert_eq!(loaded.result_payload.as_deref(), Some(r#"{"ok":true}"#));
    uow.commit().await?;

    Ok(())
}
