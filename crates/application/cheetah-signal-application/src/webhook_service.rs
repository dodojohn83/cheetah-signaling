//! Webhook configuration and delivery application service.

use crate::dto::{CreateWebhookRequest, TriggerWebhookRequest, UpdateWebhookRequest};
use cheetah_domain::{
    Clock, DeliveryStatus, DomainError, IdGenerator, WebhookConfig, WebhookDelivery,
    sign_webhook_payload,
};
use cheetah_signal_types::{
    DeliveryId, DurationMs, EventId, Page, PageRequest, RequestContext, SecretStore, UtcTimestamp,
    WebhookId,
};
use cheetah_storage_api::Storage;
use secrecy::ExposeSecret;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Configuration for webhook delivery retries and circuit breaker behavior.
#[derive(Clone, Debug)]
pub struct WebhookDeliveryConfig {
    /// Maximum delivery attempts before dead-lettering.
    pub max_attempts: u32,
    /// Initial retry backoff.
    pub base_delay_ms: DurationMs,
    /// Maximum retry backoff.
    pub max_delay_ms: DurationMs,
    /// Per-attempt HTTP request timeout.
    pub request_timeout_ms: DurationMs,
    /// Consecutive failures before a cooldown is applied.
    pub circuit_breaker_threshold: u32,
    /// Cooldown duration once the circuit breaker threshold is reached.
    pub circuit_breaker_cooldown_ms: DurationMs,
}

impl Default for WebhookDeliveryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: DurationMs::from_millis(1_000),
            max_delay_ms: DurationMs::from_millis(300_000),
            request_timeout_ms: DurationMs::from_millis(10_000),
            circuit_breaker_threshold: 3,
            circuit_breaker_cooldown_ms: DurationMs::from_millis(30_000),
        }
    }
}

/// An outbound webhook HTTP request.
#[derive(Clone, Debug)]
pub struct WebhookHttpRequest {
    /// Target URL.
    pub url: String,
    /// HTTP headers to send.
    pub headers: Vec<(String, String)>,
    /// Request body bytes.
    pub body: Vec<u8>,
    /// Per-attempt HTTP request timeout in milliseconds.
    pub timeout: Option<DurationMs>,
}

/// Response from an outbound webhook HTTP request.
#[derive(Clone, Debug)]
pub struct WebhookHttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body bytes.
    pub body: Vec<u8>,
}

/// Outbound HTTP client used to deliver webhook payloads.
#[async_trait::async_trait]
pub trait WebhookHttpClient: Send + Sync + std::fmt::Debug {
    /// Sends a webhook request and returns the response.
    async fn send(&self, request: WebhookHttpRequest) -> crate::Result<WebhookHttpResponse>;
}

/// Application service for webhook configuration and delivery.
#[derive(Clone)]
pub struct WebhookService {
    storage: Arc<dyn Storage>,
    secret_store: Arc<dyn SecretStore>,
    clock: Arc<dyn Clock>,
    id_generator: Arc<dyn IdGenerator>,
    http_client: Arc<dyn WebhookHttpClient>,
    config: WebhookDeliveryConfig,
    consecutive_failures: Arc<Mutex<HashMap<WebhookId, u32>>>,
}

impl std::fmt::Debug for WebhookService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebhookService")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl WebhookService {
    /// Creates a new webhook service.
    pub fn new(
        storage: Arc<dyn Storage>,
        secret_store: Arc<dyn SecretStore>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        http_client: Arc<dyn WebhookHttpClient>,
        config: WebhookDeliveryConfig,
    ) -> Self {
        Self {
            storage,
            secret_store,
            clock,
            id_generator,
            http_client,
            config,
            consecutive_failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Creates a new webhook configuration.
    pub async fn create_webhook(
        &self,
        context: &RequestContext,
        request: CreateWebhookRequest,
    ) -> crate::Result<WebhookConfig> {
        let config = WebhookConfig::new(
            self.clock.as_ref(),
            self.id_generator.as_ref(),
            context.tenant_id,
            request.url,
            request.secret_ref,
            request.event_types,
        )?;
        let mut uow = self.begin().await?;
        uow.webhook_config_repository().save(&config).await?;
        uow.commit().await?;
        Ok(config)
    }

    /// Lists webhook configurations for the tenant.
    pub async fn list_webhooks(
        &self,
        context: &RequestContext,
        enabled: Option<bool>,
        event_type: Option<String>,
        page: PageRequest,
    ) -> crate::Result<Page<WebhookConfig>> {
        let mut uow = self.begin().await?;
        let page = uow
            .webhook_config_repository()
            .list(context.tenant_id, enabled, event_type, page)
            .await?;
        uow.commit().await?;
        Ok(page)
    }

    /// Gets a webhook configuration by id.
    pub async fn get_webhook(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
    ) -> crate::Result<WebhookConfig> {
        let mut uow = self.begin().await?;
        let config = uow
            .webhook_config_repository()
            .get(context.tenant_id, webhook_id)
            .await?
            .ok_or_else(|| DomainError::not_found("webhook", webhook_id.to_string()))?;
        uow.commit().await?;
        Ok(config)
    }

    /// Updates a webhook configuration.
    ///
    /// `expected_revision` is the client-observed revision from `If-Match` /
    /// `ETag`. A mismatch returns a failed-precondition error (HTTP 412).
    pub async fn update_webhook(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
        expected_revision: cheetah_signal_types::Revision,
        request: UpdateWebhookRequest,
    ) -> crate::Result<WebhookConfig> {
        let mut uow = self.begin().await?;
        let mut config = uow
            .webhook_config_repository()
            .get(context.tenant_id, webhook_id)
            .await?
            .ok_or_else(|| DomainError::not_found("webhook", webhook_id.to_string()))?;
        let current = config.revision();
        if current != expected_revision {
            return Err(crate::SignalError::new(
                cheetah_signal_types::SignalErrorKind::FailedPrecondition,
                format!(
                    "webhook revision mismatch: If-Match {}, current {}",
                    expected_revision.0, current.0
                ),
            ));
        }
        config.update(
            self.clock.as_ref(),
            request.url,
            request.secret_ref,
            request.event_types,
            request.enabled,
        )?;
        uow.webhook_config_repository().save(&config).await?;
        uow.commit().await?;
        Ok(config)
    }

    /// Deletes a webhook configuration.
    pub async fn delete_webhook(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
    ) -> crate::Result<()> {
        let mut uow = self.begin().await?;
        uow.webhook_config_repository()
            .delete(context.tenant_id, webhook_id)
            .await?;
        uow.commit().await?;
        Ok(())
    }

    /// Manually triggers a test delivery for a webhook.
    pub async fn trigger_webhook(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
        request: TriggerWebhookRequest,
    ) -> crate::Result<WebhookDelivery> {
        let mut uow = self.begin().await?;
        let config = uow
            .webhook_config_repository()
            .get(context.tenant_id, webhook_id)
            .await?
            .ok_or_else(|| DomainError::not_found("webhook", webhook_id.to_string()))?;
        if !config.enabled() {
            return Err(DomainError::invalid_argument("webhook is disabled").into());
        }
        if !config.matches_event(&request.event_type) {
            return Err(DomainError::invalid_argument(
                "webhook does not subscribe to the requested event type",
            )
            .into());
        }
        let event_id = self.id_generator.generate_event_id();
        let payload = serde_json::to_vec(&request.payload)
            .map_err(|e| DomainError::invalid_argument(format!("invalid payload: {e}")))?;
        let delivery = self
            .create_and_sign_delivery(&config, event_id, payload)
            .await?;
        uow.webhook_delivery_repository().save(&delivery).await?;
        uow.commit().await?;
        Ok(delivery)
    }

    /// Lists deliveries for a webhook.
    pub async fn list_deliveries(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
        status: Option<String>,
        page: PageRequest,
    ) -> crate::Result<Page<WebhookDelivery>> {
        let mut uow = self.begin().await?;
        let page = uow
            .webhook_delivery_repository()
            .list(context.tenant_id, webhook_id, status, page)
            .await?;
        uow.commit().await?;
        Ok(page)
    }

    /// Replays a previous delivery as a new pending delivery.
    pub async fn replay_delivery(
        &self,
        context: &RequestContext,
        webhook_id: WebhookId,
        delivery_id: DeliveryId,
    ) -> crate::Result<WebhookDelivery> {
        let mut uow = self.begin().await?;
        let original = uow
            .webhook_delivery_repository()
            .get(context.tenant_id, delivery_id)
            .await?
            .ok_or_else(|| DomainError::not_found("delivery", delivery_id.to_string()))?;
        if original.webhook_id() != webhook_id {
            return Err(DomainError::not_found("delivery", delivery_id.to_string()).into());
        }
        let config = uow
            .webhook_config_repository()
            .get(context.tenant_id, original.webhook_id())
            .await?
            .ok_or_else(|| {
                DomainError::not_found("webhook_config", original.webhook_id().to_string())
            })?;
        uow.commit().await?;

        let delivery = self
            .create_and_sign_delivery(&config, original.event_id(), original.payload().to_vec())
            .await?;
        let mut uow = self.begin().await?;
        uow.webhook_delivery_repository().save(&delivery).await?;
        uow.commit().await?;
        Ok(delivery)
    }

    /// Processes pending deliveries up to `limit` and returns the number attempted.
    pub async fn process_pending(&self, limit: usize) -> crate::Result<usize> {
        let now = self.clock.now_wall();
        let mut uow = self.begin().await?;
        let pending = uow
            .webhook_delivery_repository()
            .pending(now, limit)
            .await?;
        uow.commit().await?;

        let mut attempted = 0;
        for delivery in pending {
            if let Err(e) = self.attempt_delivery(delivery).await {
                tracing::error!(error = %e, "webhook delivery attempt failed");
            }
            attempted += 1;
        }
        Ok(attempted)
    }

    async fn attempt_delivery(&self, delivery: WebhookDelivery) -> crate::Result<()> {
        if !matches!(
            delivery.status(),
            DeliveryStatus::Pending | DeliveryStatus::Failed
        ) {
            return Ok(());
        }

        let config = {
            let mut uow = self.begin().await?;
            let config = uow
                .webhook_config_repository()
                .get(delivery.tenant_id(), delivery.webhook_id())
                .await?;
            uow.commit().await?;
            config
        };

        let config = match config {
            Some(c) if c.enabled() => c,
            Some(_) => {
                let mut dl = delivery;
                dl.dead_letter(self.clock.as_ref(), "webhook disabled".to_string())?;
                self.save_delivery(dl).await?;
                return Ok(());
            }
            None => {
                let mut dl = delivery;
                dl.dead_letter(
                    self.clock.as_ref(),
                    "webhook configuration removed".to_string(),
                )?;
                self.save_delivery(dl).await?;
                return Ok(());
            }
        };

        let mut current = delivery;
        current.start_attempt(self.clock.as_ref());

        let (signature, timestamp) = match self.sign_delivery(&current, &config).await {
            Ok(pair) => pair,
            Err(e) => {
                return self
                    .handle_send_failure(current, config.webhook_id(), e)
                    .await;
            }
        };

        current.set_signature(signature.clone())?;

        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            (
                "X-Cheetah-Delivery".to_string(),
                current.delivery_id().to_string(),
            ),
            (
                "X-Cheetah-Event-Id".to_string(),
                current.event_id().to_string(),
            ),
            ("X-Cheetah-Timestamp".to_string(), timestamp.clone()),
            ("X-Cheetah-Signature".to_string(), signature),
        ];
        let secret_ref_header = config
            .secret_ref()
            .strip_prefix("sig.")
            .unwrap_or(config.secret_ref())
            .to_string();
        headers.push(("X-Cheetah-Secret-Ref".to_string(), secret_ref_header));

        let request = WebhookHttpRequest {
            url: config.url().to_string(),
            headers,
            body: current.payload().to_vec(),
            timeout: Some(self.config.request_timeout_ms),
        };

        match self.http_client.send(request).await {
            Ok(resp) if (200..300).contains(&resp.status) => {
                current.succeed(self.clock.as_ref());
                self.reset_failures(config.webhook_id());
                self.save_delivery(current).await?;
                Ok(())
            }
            Ok(resp) => {
                let error = crate::SignalError::new(
                    cheetah_signal_types::SignalErrorKind::ProtocolFailed,
                    format!("HTTP {}", resp.status),
                );
                self.handle_send_failure(current, config.webhook_id(), error)
                    .await
            }
            Err(e) => {
                self.handle_send_failure(current, config.webhook_id(), e)
                    .await
            }
        }
    }

    async fn create_and_sign_delivery(
        &self,
        config: &WebhookConfig,
        event_id: EventId,
        payload: Vec<u8>,
    ) -> crate::Result<WebhookDelivery> {
        let secret = self.secret_store.get(config.secret_ref())?;
        let timestamp = self.clock.now_wall().to_rfc3339()?;
        let mut delivery = WebhookDelivery::new(
            self.clock.as_ref(),
            self.id_generator.as_ref(),
            config.tenant_id(),
            config.webhook_id(),
            event_id,
            payload,
        )?;
        let signature = sign_webhook_payload(
            secret.expose_secret(),
            &delivery.delivery_id().to_string(),
            &delivery.event_id().to_string(),
            &timestamp,
            delivery.payload(),
        )?;
        delivery.set_signature(signature)?;
        Ok(delivery)
    }

    async fn sign_delivery(
        &self,
        delivery: &WebhookDelivery,
        config: &WebhookConfig,
    ) -> crate::Result<(String, String)> {
        let secret = self.secret_store.get(config.secret_ref())?;
        let timestamp = self.clock.now_wall().to_rfc3339()?;
        let signature = sign_webhook_payload(
            secret.expose_secret(),
            &delivery.delivery_id().to_string(),
            &delivery.event_id().to_string(),
            &timestamp,
            delivery.payload(),
        )?;
        Ok((signature, timestamp))
    }

    async fn handle_send_failure(
        &self,
        mut delivery: WebhookDelivery,
        webhook_id: WebhookId,
        error: crate::SignalError,
    ) -> crate::Result<()> {
        let message = error.message().to_string();
        if error.is_retryable() {
            self.bump_failures(webhook_id);
            self.fail_delivery(delivery, message).await
        } else {
            delivery.dead_letter(self.clock.as_ref(), message)?;
            self.save_delivery(delivery).await
        }
    }

    async fn fail_delivery(
        &self,
        mut delivery: WebhookDelivery,
        error: String,
    ) -> crate::Result<()> {
        let now = self.clock.now_wall();
        let next_attempt = self.compute_next_attempt(now, delivery.attempt_count());
        let webhook_id = delivery.webhook_id();
        let next_attempt = self.apply_circuit_breaker(now, webhook_id, next_attempt);

        if delivery.attempt_count() >= self.config.max_attempts {
            delivery.dead_letter(self.clock.as_ref(), error)?;
        } else {
            delivery.fail(self.clock.as_ref(), error, next_attempt)?;
        }

        self.save_delivery(delivery).await
    }

    async fn save_delivery(&self, delivery: WebhookDelivery) -> crate::Result<()> {
        let mut uow = self.begin().await?;
        uow.webhook_delivery_repository().save(&delivery).await?;
        uow.commit().await?;
        Ok(())
    }

    async fn begin(&self) -> crate::Result<Box<dyn cheetah_domain::UnitOfWork>> {
        self.storage
            .begin()
            .await
            .map_err(DomainError::from)
            .map_err(crate::SignalError::from)
    }

    fn compute_next_attempt(&self, now: UtcTimestamp, attempt_count: u32) -> Option<UtcTimestamp> {
        let shift = attempt_count.min(30) as i64;
        let backoff_ms = self
            .config
            .base_delay_ms
            .as_millis()
            .saturating_mul(1i64 << shift)
            .min(self.config.max_delay_ms.as_millis());
        now.checked_add(DurationMs::from_millis(backoff_ms))
    }

    fn apply_circuit_breaker(
        &self,
        now: UtcTimestamp,
        webhook_id: WebhookId,
        next_attempt: Option<UtcTimestamp>,
    ) -> Option<UtcTimestamp> {
        let count = {
            let map = self.lock_failures();
            *map.get(&webhook_id).unwrap_or(&0)
        };
        if self.config.circuit_breaker_threshold > 0
            && count > 0
            && count % self.config.circuit_breaker_threshold == 0
        {
            let cooldown = now
                .checked_add(self.config.circuit_breaker_cooldown_ms)
                .unwrap_or(now);
            Some(cooldown.max(next_attempt.unwrap_or(cooldown)))
        } else {
            next_attempt
        }
    }

    fn reset_failures(&self, webhook_id: WebhookId) {
        let mut map = self.lock_failures();
        map.insert(webhook_id, 0);
    }

    fn bump_failures(&self, webhook_id: WebhookId) {
        let mut map = self.lock_failures();
        *map.entry(webhook_id).or_insert(0) += 1;
    }

    fn lock_failures(&self) -> std::sync::MutexGuard<'_, HashMap<WebhookId, u32>> {
        match self.consecutive_failures.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}
