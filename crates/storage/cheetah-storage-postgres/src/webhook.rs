//! Webhook repository implementations for the PostgreSQL storage adapter.

pub(crate) type Db = ::sqlx::Postgres;

include!("../../cheetah-storage-api/include/webhook_impl.rs");

use crate::unit_of_work::PostgresUnitOfWork;
use cheetah_domain::{WebhookConfigRepository, WebhookDeliveryRepository};

#[async_trait::async_trait]
impl WebhookConfigRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
    ) -> cheetah_domain::Result<Option<cheetah_domain::WebhookConfig>> {
        get_webhook_config(self.tx()?.as_mut(), tenant_id, webhook_id).await
    }

    async fn save(&mut self, config: &cheetah_domain::WebhookConfig) -> cheetah_domain::Result<()> {
        save_webhook_config(self.tx()?.as_mut(), config).await
    }

    async fn delete(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
    ) -> cheetah_domain::Result<()> {
        delete_webhook_config(self.tx()?.as_mut(), tenant_id, webhook_id).await
    }

    async fn list(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        enabled: Option<bool>,
        event_type: Option<String>,
        page: cheetah_signal_types::PageRequest,
    ) -> cheetah_domain::Result<cheetah_signal_types::Page<cheetah_domain::WebhookConfig>> {
        list_webhook_configs(self.tx()?.as_mut(), tenant_id, enabled, event_type, page).await
    }
}

#[async_trait::async_trait]
impl WebhookDeliveryRepository for PostgresUnitOfWork {
    async fn get(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        delivery_id: cheetah_signal_types::DeliveryId,
    ) -> cheetah_domain::Result<Option<cheetah_domain::WebhookDelivery>> {
        get_webhook_delivery(self.tx()?.as_mut(), tenant_id, delivery_id).await
    }

    async fn save(
        &mut self,
        delivery: &cheetah_domain::WebhookDelivery,
    ) -> cheetah_domain::Result<()> {
        save_webhook_delivery(self.tx()?.as_mut(), delivery).await
    }

    async fn list(
        &mut self,
        tenant_id: cheetah_signal_types::TenantId,
        webhook_id: cheetah_signal_types::WebhookId,
        status: Option<String>,
        page: cheetah_signal_types::PageRequest,
    ) -> cheetah_domain::Result<cheetah_signal_types::Page<cheetah_domain::WebhookDelivery>> {
        list_webhook_deliveries(self.tx()?.as_mut(), tenant_id, webhook_id, status, page).await
    }

    async fn pending(
        &mut self,
        now: cheetah_signal_types::UtcTimestamp,
        limit: usize,
    ) -> cheetah_domain::Result<Vec<cheetah_domain::WebhookDelivery>> {
        pending_webhook_deliveries(self.tx()?.as_mut(), now, limit).await
    }
}
