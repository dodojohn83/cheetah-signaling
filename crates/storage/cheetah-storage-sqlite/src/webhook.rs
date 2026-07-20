//! Webhook repository implementations for the SQLite storage adapter.

pub(crate) type Db = ::sqlx::Sqlite;
pub(crate) const IS_POSTGRES: bool = false;

include!("../../cheetah-storage-api/include/webhook_impl.rs");

use crate::unit_of_work::SqliteUnitOfWork;
use cheetah_domain::{WebhookConfigRepository, WebhookDeliveryRepository};

#[async_trait::async_trait]
impl WebhookConfigRepository for SqliteUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
    ) -> cheetah_domain::Result<Option<cheetah_domain::WebhookConfig>> {
        get_webhook_config(self.tx().await?.as_mut(), tenant_id, webhook_id).await
    }

    async fn save(&mut self, config: &cheetah_domain::WebhookConfig) -> cheetah_domain::Result<()> {
        save_webhook_config(self.tx().await?.as_mut(), config).await
    }

    async fn delete(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
    ) -> cheetah_domain::Result<()> {
        delete_webhook_config(self.tx().await?.as_mut(), tenant_id, webhook_id).await
    }

    async fn list(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        enabled: Option<bool>,
        event_type: Option<String>,
        page: cheetah_signal_types::PageRequest,
    ) -> cheetah_domain::Result<cheetah_signal_types::Page<cheetah_domain::WebhookConfig>> {
        list_webhook_configs(
            self.tx().await?.as_mut(),
            tenant_id,
            enabled,
            event_type,
            page,
        )
        .await
    }
}

#[async_trait::async_trait]
impl WebhookDeliveryRepository for SqliteUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        delivery_id: cheetah_signal_types::DeliveryId,
    ) -> cheetah_domain::Result<Option<cheetah_domain::WebhookDelivery>> {
        get_webhook_delivery(self.tx().await?.as_mut(), tenant_id, delivery_id).await
    }

    async fn save(
        &mut self,
        delivery: &cheetah_domain::WebhookDelivery,
    ) -> cheetah_domain::Result<()> {
        save_webhook_delivery(self.tx().await?.as_mut(), delivery).await
    }

    async fn list(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
        status: Option<String>,
        page: cheetah_signal_types::PageRequest,
    ) -> cheetah_domain::Result<cheetah_signal_types::Page<cheetah_domain::WebhookDelivery>> {
        list_webhook_deliveries(
            self.tx().await?.as_mut(),
            tenant_id,
            webhook_id,
            status,
            page,
        )
        .await
    }

    async fn pending(
        &mut self,
        now: cheetah_signal_types::UtcTimestamp,
        limit: usize,
    ) -> cheetah_domain::Result<Vec<cheetah_domain::WebhookDelivery>> {
        pending_webhook_deliveries(self.tx().await?.as_mut(), now, limit).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::SqliteStorage;
    use cheetah_domain::WebhookConfig;
    use cheetah_signal_types::IdGenerator;
    use cheetah_signal_types::test_support::{FakeClock, FakeIdGenerator};
    use cheetah_storage_api::Storage;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct RemoveOnDrop(PathBuf);
    impl Drop for RemoveOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[tokio::test]
    async fn save_webhook_config_rejects_negative_stored_revision() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("cheetah-webhook-negative-revision-{suffix}.db"));
        let _guard = RemoveOnDrop(path.clone());

        let storage = SqliteStorage::new(&path).await.unwrap();
        storage.migration().run().await.unwrap();

        let clock = FakeClock::new();
        let id_generator = FakeIdGenerator::new();
        let tenant_id = id_generator.generate_tenant_id();
        let config = WebhookConfig::new(
            &clock,
            &id_generator,
            tenant_id,
            "https://example.com/webhook".to_string(),
            "secret://test".to_string(),
            vec!["device.online".to_string()],
        )
        .unwrap();

        let mut uow = storage.begin().await.unwrap();
        uow.webhook_config_repository().save(&config).await.unwrap();
        uow.commit().await.unwrap();

        // Simulate database corruption: set the stored revision to a negative value.
        {
            let mut conn = storage.write_pool().acquire().await.unwrap();
            sqlx::query(
                "UPDATE webhook_configs SET revision = -1 WHERE tenant_id = ? AND webhook_id = ?",
            )
            .bind(tenant_id.as_uuid())
            .bind(config.webhook_id().as_uuid())
            .execute(&mut *conn)
            .await
            .unwrap();
        }

        let mut stale = config.clone();
        stale
            .update(
                &clock,
                Some("https://example.com/updated".to_string()),
                None,
                None,
                None,
            )
            .unwrap();

        let mut uow = storage.begin().await.unwrap();
        let result = uow.webhook_config_repository().save(&stale).await;
        assert!(
            matches!(result, Err(cheetah_domain::DomainError::Internal { .. })),
            "expected internal error for corrupt negative stored revision, got {:?}",
            result
        );
    }
}
