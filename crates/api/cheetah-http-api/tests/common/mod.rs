//! Shared test harness for `cheetah-http-api` integration tests.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryDeviceOwnerResolver, InMemoryIdGenerator, InMemoryMediaPort,
};
use cheetah_http_api::{ApiConfig, ApiServer, ApiState};
use cheetah_message_local::InProcessMessageBus;
use cheetah_signal_types::IdGenerator;
use cheetah_signal_types::config::SecurityConfig;
use cheetah_storage_api::Storage;
use secrecy::SecretString;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// A running HTTP API server and its client for tests.
pub struct TestServer {
    base_url: String,
    api_key: String,
    tenant_id: String,
    _server: ApiServer,
    _temp_dir: PathBuf,
    client: reqwest::Client,
}

impl TestServer {
    /// Starts a new server on a random local port with an empty SQLite database.
    pub async fn new() -> Self {
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

        let api_key = "test-api-key".to_string();
        let tenant_id = uuid::Uuid::now_v7().to_string();
        let security = SecurityConfig {
            static_api_key: SecretString::from(api_key.clone()),
            ..Default::default()
        };
        let config = ApiConfig {
            listen_addr: "127.0.0.1".to_string(),
            port: 0,
            read_timeout_ms: 5000,
            request_body_limit_bytes: 1024 * 1024,
            node_id,
            security,
        };

        let state = ApiState::new(
            config,
            storage,
            Arc::new(InMemoryClock::new()),
            Arc::clone(&id_generator),
            Arc::new(InProcessMessageBus::new(64, 256)),
            Arc::new(InMemoryDeviceOwnerResolver::new()),
            Arc::new(InMemoryMediaPort::new(Arc::clone(&id_generator))),
        );

        let server = ApiServer::start(state).await.expect("start server");
        let SocketAddr::V4(addr) = server.local_addr else {
            panic!("expected ipv4 address");
        };
        let base_url = format!("http://127.0.0.1:{}", addr.port());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("build client");

        Self {
            base_url,
            api_key,
            tenant_id,
            _server: server,
            _temp_dir: temp_dir,
            client,
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

    /// Returns an authenticated request builder for the given method and path.
    pub fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.client
            .request(method, url)
            .header("x-api-key", &self.api_key)
            .header("x-tenant-id", &self.tenant_id)
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
