//! Shared test harness for `cheetah-http-api` integration tests.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    missing_debug_implementations
)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
};
use cheetah_http_api::{ApiConfig, ApiServer, ApiState};
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_application::{
    WebhookDeliveryConfig, WebhookHttpClient, WebhookHttpRequest, WebhookHttpResponse,
};
use cheetah_signal_types::config::{LogFormat, SecurityConfig};
use cheetah_signal_types::{DurationMs, IdGenerator, SecretStore, SignalError, SignalErrorKind};
use cheetah_storage_api::Storage;
use secrecy::SecretString;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Options controlling how [`TestServer`] is assembled.
///
/// Defaults disable rate limiting and JWT, and enable the static API key.
#[derive(Clone, Debug, Default)]
pub struct TestServerOptions {
    /// Token-bucket requests per second. Zero disables rate limiting.
    pub rate_limit_requests_per_second: u32,
    /// Token-bucket burst capacity. Zero disables rate limiting.
    pub rate_limit_burst: u32,
    /// Optional PEM-encoded RSA public key for JWT validation.
    pub jwt_public_key_pem: Option<String>,
    /// JWT audience claims accepted by the server.
    pub jwt_audience: Vec<String>,
    /// JWT issuer claims accepted by the server.
    pub jwt_issuer: Vec<String>,
    /// When true, leave `static_api_key` empty so only JWT auth is available.
    pub disable_static_api_key: bool,
}

/// A running HTTP API server and its client for tests.
pub struct TestServer {
    base_url: String,
    api_key: String,
    tenant_id: String,
    _server: ApiServer,
    _temp_dir: PathBuf,
    client: reqwest::Client,
    webhook_requests: Arc<Mutex<Vec<WebhookHttpRequest>>>,
}

#[derive(Debug)]
struct TestSecretStore {
    secrets: Mutex<HashMap<String, SecretString>>,
}

impl TestSecretStore {
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

impl SecretStore for TestSecretStore {
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

    fn rotate(&self, key: &str) -> cheetah_signal_types::Result<SecretString> {
        let _ = key;
        Ok(SecretString::from("rotated"))
    }
}

#[derive(Debug, Default)]
struct TestWebhookHttpClient {
    requests: Arc<Mutex<Vec<WebhookHttpRequest>>>,
}

#[async_trait::async_trait]
impl WebhookHttpClient for TestWebhookHttpClient {
    async fn send(
        &self,
        request: WebhookHttpRequest,
    ) -> cheetah_signal_types::Result<WebhookHttpResponse> {
        let mut requests = self.requests.lock().unwrap();
        requests.push(request);
        Ok(WebhookHttpResponse {
            status: 200,
            body: Vec::new(),
        })
    }
}

impl TestServer {
    /// Starts a new server on a random local port with an empty SQLite database.
    pub async fn new() -> Self {
        Self::with_options(TestServerOptions::default()).await
    }

    /// Starts a server with custom rate-limit / auth options.
    pub async fn with_options(options: TestServerOptions) -> Self {
        let id_generator: Arc<dyn IdGenerator> = Arc::new(InMemoryIdGenerator::new());
        let node_id = id_generator.generate_node_id();
        let temp_suffix = format!("{}-{}", node_id, uuid::Uuid::now_v7());

        let temp_dir = std::env::temp_dir().join(format!("cheetah-http-api-test-{temp_suffix}"));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).expect("create temp dir");
        let db_path = temp_dir.join("cheetah.db");

        let storage = Arc::new(
            cheetah_storage_sqlite::SqliteStorage::new(&db_path)
                .await
                .expect("create sqlite storage"),
        );
        storage.migration().run().await.expect("run migrations");

        let api_key = if options.disable_static_api_key {
            String::new()
        } else {
            "test-api-key-with-at-least-32-characters".to_string()
        };
        let tenant_id = uuid::Uuid::now_v7().to_string();
        let security = SecurityConfig {
            static_api_key: SecretString::from(api_key.clone()),
            jwt_public_key_ref: SecretString::from(
                options.jwt_public_key_pem.clone().unwrap_or_default(),
            ),
            jwt_audience: options.jwt_audience.clone(),
            jwt_issuer: options.jwt_issuer.clone(),
            ..Default::default()
        };
        let config = ApiConfig {
            listen_addr: "127.0.0.1".to_string(),
            port: 0,
            read_timeout_ms: 5000,
            request_body_limit_bytes: 1024 * 1024,
            cors_allowed_origins: Vec::new(),
            rate_limit_requests_per_second: options.rate_limit_requests_per_second,
            rate_limit_burst: options.rate_limit_burst,
            webhook_delivery_interval_ms: 0,
            node_id,
            security,
            log_level: "info".to_string(),
            log_format: LogFormat::Json,
            protocol_body_logging: false,
            media_nodes_required: false,
        };

        cheetah_http_api::logging::init_tracing(&config.log_level, config.log_format);

        let secret_store = Arc::new(TestSecretStore::new());
        secret_store.insert("sig.test", "super-secret");

        let webhook_requests = Arc::new(Mutex::new(Vec::new()));
        let http_client = Arc::new(TestWebhookHttpClient {
            requests: webhook_requests.clone(),
        });

        let gb_metrics = Arc::new(cheetah_runtime_tokio::GbMetrics::new(4, 1024));
        let state = ApiState::new(
            config,
            storage,
            Arc::new(InMemoryClock::new()),
            Arc::clone(&id_generator),
            Arc::new(InProcessMessageBus::new(64, 256)),
            Arc::new(InMemoryDeviceOwnerResolver::new()),
            Arc::new(InMemoryMediaPort::new(Arc::clone(&id_generator))),
        )
        .with_gb_metrics(gb_metrics.clone())
        .with_runtime_health(gb_metrics)
        .with_webhook_service(
            secret_store,
            http_client,
            WebhookDeliveryConfig {
                max_attempts: 5,
                base_delay_ms: DurationMs::from_millis(100),
                max_delay_ms: DurationMs::from_millis(1_000),
                request_timeout_ms: DurationMs::from_millis(1_000),
                circuit_breaker_threshold: 3,
                circuit_breaker_cooldown_ms: DurationMs::from_millis(2_000),
            },
        );

        let server = ApiServer::start(state).await.expect("start server");
        let SocketAddr::V4(addr) = server.local_addr else {
            panic!("expected ipv4 address");
        };
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        // Bypass ambient HTTP(S)_PROXY so tests hit the local listener directly.
        // Environments often set no_proxy=localhost but not 127.0.0.1.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .expect("build client");

        Self {
            base_url,
            api_key,
            tenant_id,
            _server: server,
            _temp_dir: temp_dir,
            client,
            webhook_requests,
        }
    }

    /// Returns the base URL of the server.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Returns the static API key used for authentication.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Returns the default tenant identifier used for requests.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// Returns captured webhook HTTP requests, if any.
    pub fn webhook_requests(&self) -> Vec<WebhookHttpRequest> {
        let mut requests = self.webhook_requests.lock().unwrap();
        std::mem::take(&mut *requests)
    }

    /// Returns an authenticated request builder for the given method and path.
    pub fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let mut builder = self.client.request(method, url);
        if !self.api_key.is_empty() {
            builder = builder.header("x-api-key", &self.api_key);
        }
        builder.header("x-tenant-id", &self.tenant_id)
    }

    /// Returns an unauthenticated request builder for the given method and path.
    pub fn request_unauthenticated(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.client.request(method, url)
    }
}
