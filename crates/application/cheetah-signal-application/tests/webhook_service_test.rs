//! Webhook service integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_domain::{Clock, IdGenerator, MAX_WEBHOOK_EVENT_TYPE_BYTES};
use cheetah_signal_application::{
    CreateWebhookRequest, TriggerWebhookRequest, UpdateWebhookRequest, WebhookDeliveryConfig,
    WebhookHttpClient, WebhookHttpRequest, WebhookHttpResponse, WebhookService,
};
use cheetah_signal_types::{DurationMs, SecretStore, SignalError, SignalErrorKind};
use cheetah_storage_api::Storage;
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
struct FakeSecretStore {
    secrets: Mutex<HashMap<String, SecretString>>,
}

impl FakeSecretStore {
    fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }

    fn insert(&self, key: &str, value: &str) {
        let mut secrets = self.secrets.lock().unwrap();
        secrets.insert(key.to_string(), SecretString::from(value.to_string()));
    }
}

impl SecretStore for FakeSecretStore {
    fn get(&self, key: &str) -> cheetah_signal_types::Result<SecretString> {
        let secrets = self.secrets.lock().unwrap();
        secrets
            .get(key)
            .cloned()
            .ok_or_else(|| SignalError::new(SignalErrorKind::NotFound, "secret not found"))
    }

    fn put(&self, key: &str, value: SecretString) -> cheetah_signal_types::Result<()> {
        let mut secrets = self.secrets.lock().unwrap();
        secrets.insert(key.to_string(), value);
        Ok(())
    }

    fn delete(&self, key: &str) -> cheetah_signal_types::Result<()> {
        let mut secrets = self.secrets.lock().unwrap();
        secrets.remove(key);
        Ok(())
    }

    fn rotate(&self, _key: &str) -> cheetah_signal_types::Result<SecretString> {
        Ok(SecretString::from("rotated"))
    }
}

#[derive(Debug, Default)]
struct FakeWebhookHttpClient {
    requests: Arc<Mutex<Vec<WebhookHttpRequest>>>,
    responses: Arc<Mutex<Vec<cheetah_signal_types::Result<WebhookHttpResponse>>>>,
}

impl FakeWebhookHttpClient {
    fn push_ok(&self, status: u16) {
        let mut responses = self.responses.lock().unwrap();
        responses.push(Ok(WebhookHttpResponse {
            status,
            body: Vec::new(),
        }));
    }

    fn push_err(&self, kind: SignalErrorKind, message: &str) {
        let mut responses = self.responses.lock().unwrap();
        responses.push(Err(SignalError::new(kind, message.to_string())));
    }

    fn take_requests(&self) -> Vec<WebhookHttpRequest> {
        let mut requests = self.requests.lock().unwrap();
        std::mem::take(&mut *requests)
    }
}

#[async_trait::async_trait]
impl WebhookHttpClient for FakeWebhookHttpClient {
    async fn send(
        &self,
        request: WebhookHttpRequest,
    ) -> cheetah_signal_types::Result<WebhookHttpResponse> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            return Err(SignalError::new(
                SignalErrorKind::Unavailable,
                "no response configured",
            ));
        }
        responses.remove(0)
    }
}

struct TestContext {
    tenant_id: cheetah_signal_types::TenantId,
    clock: Arc<InMemoryClock>,
    id_generator: Arc<InMemoryIdGenerator>,
    secret_store: Arc<FakeSecretStore>,
    http_client: Arc<FakeWebhookHttpClient>,
    service: WebhookService,
    _temp_dir: std::path::PathBuf,
}

async fn setup() -> TestContext {
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let clock: Arc<InMemoryClock> = Arc::new(InMemoryClock::new());
    let tenant_id = id_generator.generate_tenant_id();

    let temp_dir = std::env::temp_dir().join(format!(
        "webhook-test-{}-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::SeqCst),
        id_generator.generate_message_id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("cheetah.db");

    let storage = Arc::new(
        cheetah_storage_sqlite::SqliteStorage::new(&db_path)
            .await
            .expect("create sqlite storage"),
    );
    storage.migration().run().await.expect("run migrations");

    let secret_store = Arc::new(FakeSecretStore::new());
    secret_store.insert("sig.test", "super-secret");

    let http_client = Arc::new(FakeWebhookHttpClient::default());

    let service = WebhookService::new(
        storage.clone(),
        secret_store.clone(),
        clock.clone(),
        id_generator.clone(),
        http_client.clone(),
        WebhookDeliveryConfig {
            max_attempts: 5,
            base_delay_ms: DurationMs::from_millis(100),
            max_delay_ms: DurationMs::from_millis(1_000),
            request_timeout_ms: DurationMs::from_millis(1_000),
            circuit_breaker_threshold: 3,
            circuit_breaker_cooldown_ms: DurationMs::from_millis(2_000),
        },
    );

    TestContext {
        tenant_id,
        clock,
        id_generator,
        secret_store,
        http_client,
        service,
        _temp_dir: temp_dir,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_create_list_and_update_round_trip() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    assert_eq!(config.url(), "http://example.com/webhook");
    assert!(config.enabled());

    let page = ctx
        .service
        .list_webhooks(
            &context,
            None,
            None,
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].url(), "http://example.com/webhook");

    let updated = ctx
        .service
        .update_webhook(
            &context,
            config.webhook_id(),
            config.revision(),
            UpdateWebhookRequest {
                url: Some("http://example.com/webhook-v2".to_string()),
                secret_ref: None,
                event_types: Some(vec!["device.offline".to_string()]),
                enabled: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.url(), "http://example.com/webhook-v2");
    assert!(updated.enabled());
    assert!(updated.matches_event("device.offline"));
    assert!(!updated.matches_event("device.online"));
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_trigger_and_delivery_succeeds() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    ctx.http_client.push_ok(200);
    let delivery = ctx
        .service
        .trigger_webhook(
            &context,
            config.webhook_id(),
            TriggerWebhookRequest {
                event_type: "device.online".to_string(),
                payload: serde_json::json!({"device_id": "123"}),
            },
        )
        .await
        .unwrap();

    assert_eq!(delivery.status(), cheetah_domain::DeliveryStatus::Pending);
    assert!(!delivery.signature().is_empty());

    ctx.service.process_pending(10).await.unwrap();

    let deliveries = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            Some("succeeded".to_string()),
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deliveries.items.len(), 1);
    assert_eq!(
        deliveries.items[0].status(),
        cheetah_domain::DeliveryStatus::Succeeded
    );

    let requests = ctx.http_client.take_requests();
    assert_eq!(requests.len(), 1);
    let signature_header = requests[0]
        .headers
        .iter()
        .find(|(k, _)| k == "X-Cheetah-Signature")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_eq!(signature_header, delivery.signature());

    let secret = ctx.secret_store.get("sig.test").unwrap();
    let computed = cheetah_domain::sign_webhook_payload(
        secret.expose_secret(),
        &delivery.delivery_id().to_string(),
        &delivery.event_id().to_string(),
        &requests[0]
            .headers
            .iter()
            .find(|(k, _)| k == "X-Cheetah-Timestamp")
            .map(|(_, v)| v.clone())
            .unwrap(),
        &requests[0].body,
    )
    .unwrap();
    assert_eq!(computed, delivery.signature());
}

#[tokio::test(flavor = "current_thread")]
async fn trigger_webhook_rejects_oversized_event_type() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    let result = ctx
        .service
        .trigger_webhook(
            &context,
            config.webhook_id(),
            TriggerWebhookRequest {
                event_type: "x".repeat(MAX_WEBHOOK_EVENT_TYPE_BYTES + 1),
                payload: serde_json::json!({"x": 1}),
            },
        )
        .await;
    assert!(result.is_err(), "oversized event_type must be rejected");
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_delivery_retries_and_dead_letters() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    ctx.http_client
        .push_err(SignalErrorKind::Unavailable, "network error");
    ctx.http_client
        .push_err(SignalErrorKind::Unavailable, "network error");
    ctx.http_client
        .push_err(SignalErrorKind::Unavailable, "network error");

    let delivery = ctx
        .service
        .trigger_webhook(
            &context,
            config.webhook_id(),
            TriggerWebhookRequest {
                event_type: "device.online".to_string(),
                payload: serde_json::json!({"x": 1}),
            },
        )
        .await
        .unwrap();

    // First attempt fails.
    ctx.service.process_pending(10).await.unwrap();
    let d1 = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            None,
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.delivery_id() == delivery.delivery_id())
        .unwrap();
    assert_eq!(d1.attempt_count(), 1);
    assert_eq!(d1.status(), cheetah_domain::DeliveryStatus::Failed);
    assert!(d1.next_attempt_at().is_some());

    // Advance past the exponential backoff.
    ctx.clock.advance(DurationMs::from_millis(500));

    // Second attempt also fails.
    ctx.service.process_pending(10).await.unwrap();
    let d2 = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            None,
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.delivery_id() == delivery.delivery_id())
        .unwrap();
    assert_eq!(d2.attempt_count(), 2);
    assert_eq!(d2.status(), cheetah_domain::DeliveryStatus::Failed);

    // Third failure triggers the circuit breaker cooldown.
    ctx.clock.advance(DurationMs::from_millis(1_500));
    ctx.service.process_pending(10).await.unwrap();
    let d3 = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            None,
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.delivery_id() == delivery.delivery_id())
        .unwrap();
    assert_eq!(d3.attempt_count(), 3);
    assert_eq!(d3.status(), cheetah_domain::DeliveryStatus::Failed);
    assert!(d3.next_attempt_at().unwrap() > ctx.clock.now_wall());

    // Advance through cooldown and succeed.
    ctx.clock.advance(DurationMs::from_millis(2_500));
    let _ = ctx.http_client.take_requests();
    ctx.http_client.push_ok(200);
    ctx.service.process_pending(10).await.unwrap();
    let d4 = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            Some("succeeded".to_string()),
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.delivery_id() == delivery.delivery_id())
        .unwrap();
    assert_eq!(d4.status(), cheetah_domain::DeliveryStatus::Succeeded);
    assert_eq!(ctx.http_client.take_requests().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_delivery_dead_letters_after_max_attempts() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    for _ in 0..5 {
        ctx.http_client
            .push_err(SignalErrorKind::Unavailable, "network error");
    }

    let delivery = ctx
        .service
        .trigger_webhook(
            &context,
            config.webhook_id(),
            TriggerWebhookRequest {
                event_type: "device.online".to_string(),
                payload: serde_json::json!({"x": 1}),
            },
        )
        .await
        .unwrap();

    for i in 0..5 {
        if i > 0 {
            ctx.clock.advance(DurationMs::from_millis(2_000));
        }
        ctx.service.process_pending(10).await.unwrap();
    }

    let final_delivery = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            None,
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|d| d.delivery_id() == delivery.delivery_id())
        .unwrap();
    assert_eq!(
        final_delivery.status(),
        cheetah_domain::DeliveryStatus::DeadLetter
    );
    assert!(final_delivery.last_error().is_some());
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_replay_creates_new_pending_delivery() {
    let ctx = setup().await;
    let context = request_context(ctx.tenant_id, &*ctx.id_generator, &*ctx.clock);

    let config = ctx
        .service
        .create_webhook(
            &context,
            CreateWebhookRequest {
                url: "http://example.com/webhook".to_string(),
                secret_ref: "sig.test".to_string(),
                event_types: vec!["device.online".to_string()],
            },
        )
        .await
        .unwrap();

    ctx.http_client.push_ok(200);
    let delivery = ctx
        .service
        .trigger_webhook(
            &context,
            config.webhook_id(),
            TriggerWebhookRequest {
                event_type: "device.online".to_string(),
                payload: serde_json::json!({"x": 1}),
            },
        )
        .await
        .unwrap();
    ctx.service.process_pending(10).await.unwrap();

    ctx.http_client.push_ok(200);
    let replay = ctx
        .service
        .replay_delivery(&context, config.webhook_id(), delivery.delivery_id())
        .await
        .unwrap();
    assert_ne!(replay.delivery_id(), delivery.delivery_id());
    assert_eq!(replay.webhook_id(), delivery.webhook_id());
    assert_eq!(replay.payload(), delivery.payload());
    assert_eq!(replay.status(), cheetah_domain::DeliveryStatus::Pending);

    ctx.service.process_pending(10).await.unwrap();
    let succeeded = ctx
        .service
        .list_deliveries(
            &context,
            config.webhook_id(),
            Some("succeeded".to_string()),
            cheetah_signal_types::PageRequest::new(20).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(succeeded.items.len(), 2);
}
