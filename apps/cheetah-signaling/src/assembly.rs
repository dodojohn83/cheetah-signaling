//! Process assembly for edge and cluster profiles.
//!
//! Startup order (per AGENTS.md): config/secret → schema check → bus →
//! repository → ownership → media → protocol → public listener → ready.

use crate::gb_event_sink;
use cheetah_cluster_ownership::lease::CachingDeviceOwnerResolver;
use cheetah_domain::ports::{DeviceOwnerResolver, MediaPort};
use cheetah_domain::{DomainEvent, EventPublisher};
use cheetah_gb28181_driver_tokio::Gb28181UdpDriver;
use cheetah_gb28181_driver_tokio::config::DriverConfig as GbDriverConfig;
use cheetah_gb28181_module::config::{AuthPolicy, Gb28181DomainConfig};
use cheetah_gb28181_module::ports::{CredentialError, CredentialProvider};
use cheetah_gb28181_module::types::DeviceId as GbDeviceId;
use cheetah_http_api::state::{ApiConfig, ApiServer, ApiState};
use cheetah_media_client::{MediaClientConfig, MediaControlClient};
use cheetah_media_scheduler::{
    InMemoryMediaNodeRegistry, LeastLoadedScheduler, MediaRegistryConfig, SchedulerConfig,
    SchedulerMediaPort,
};
use cheetah_message_api::RawEventBus;
use cheetah_message_api::publisher::publish_domain_event;
use cheetah_message_local::InProcessMessageBus;
use cheetah_message_nats::NatsBus;
use cheetah_secret::{CompositeSecretStore, EnvSecretStore, FileSecretStore};
use cheetah_signal_application::OutboxRelay;
use cheetah_signal_types::config::{MessagingBackend, SignalConfig, StorageBackend};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, DurationMs, Event, IdGenerator, MediaBindingId, MediaSessionId,
    NodeId, SecretStore, TenantId, UtcTimestamp,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use time::{OffsetDateTime, UtcOffset};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

/// Running signaling process handles.
pub struct SignalingRuntime {
    /// HTTP API server handle.
    pub http: ApiServer,
    /// Shared cancellation token for background workers.
    pub cancel: CancellationToken,
    /// Bound HTTP address.
    pub http_addr: SocketAddr,
    /// Optional bound GB28181 SIP address.
    pub gb28181_addr: Option<SocketAddr>,
    /// Background worker handles (outbox, protocol drivers).
    workers: Vec<JoinHandle<()>>,
}

impl SignalingRuntime {
    /// Stops background workers and the HTTP server.
    pub fn shutdown(self) {
        self.cancel.cancel();
        for handle in self.workers {
            handle.abort();
        }
        self.http.shutdown();
    }
}

/// System wall/monotonic clock for production.
#[derive(Debug, Clone)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Creates a new system clock.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_wall(&self) -> UtcTimestamp {
        let dt = OffsetDateTime::from(SystemTime::now());
        UtcTimestamp::from_offset(dt.to_offset(UtcOffset::UTC))
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::from_millis(Instant::now().duration_since(self.start).as_millis() as i64)
    }
}

/// UUIDv7 identifier generator.
#[derive(Debug, Default, Clone, Copy)]
pub struct UuidIdGenerator;

impl IdGenerator for UuidIdGenerator {
    fn generate_tenant_id(&self) -> cheetah_signal_types::TenantId {
        cheetah_signal_types::TenantId::from_uuid(Uuid::now_v7())
    }
    fn generate_device_id(&self) -> DeviceId {
        DeviceId::from_uuid(Uuid::now_v7())
    }
    fn generate_endpoint_id(&self) -> cheetah_signal_types::EndpointId {
        cheetah_signal_types::EndpointId::from_uuid(Uuid::now_v7())
    }
    fn generate_channel_id(&self) -> ChannelId {
        ChannelId::from_uuid(Uuid::now_v7())
    }
    fn generate_protocol_session_id(&self) -> cheetah_signal_types::ProtocolSessionId {
        cheetah_signal_types::ProtocolSessionId::from_uuid(Uuid::now_v7())
    }
    fn generate_media_session_id(&self) -> MediaSessionId {
        MediaSessionId::from_uuid(Uuid::now_v7())
    }
    fn generate_media_binding_id(&self) -> MediaBindingId {
        MediaBindingId::from_uuid(Uuid::now_v7())
    }
    fn generate_media_node_instance_epoch(&self) -> cheetah_signal_types::MediaNodeInstanceEpoch {
        cheetah_signal_types::MediaNodeInstanceEpoch(Uuid::now_v7().as_u128() as u64)
    }
    fn generate_operation_id(&self) -> cheetah_signal_types::OperationId {
        cheetah_signal_types::OperationId::from_uuid(Uuid::now_v7())
    }
    fn generate_node_id(&self) -> NodeId {
        NodeId::from_uuid(Uuid::now_v7())
    }
    fn generate_node_instance_id(&self) -> cheetah_signal_types::NodeInstanceId {
        cheetah_signal_types::NodeInstanceId::from_uuid(Uuid::now_v7())
    }
    fn generate_plugin_id(&self) -> cheetah_signal_types::PluginId {
        cheetah_signal_types::PluginId::from_uuid(Uuid::now_v7())
    }
    fn generate_event_id(&self) -> cheetah_signal_types::EventId {
        cheetah_signal_types::EventId::from_uuid(Uuid::now_v7())
    }
    fn generate_message_id(&self) -> cheetah_signal_types::MessageId {
        cheetah_signal_types::MessageId::from_uuid(Uuid::now_v7())
    }
    fn generate_correlation_id(&self) -> cheetah_signal_types::CorrelationId {
        cheetah_signal_types::CorrelationId::from_uuid(Uuid::now_v7())
    }
    fn generate_webhook_id(&self) -> cheetah_signal_types::WebhookId {
        cheetah_signal_types::WebhookId::from_uuid(Uuid::now_v7())
    }
    fn generate_delivery_id(&self) -> cheetah_signal_types::DeliveryId {
        cheetah_signal_types::DeliveryId::from_uuid(Uuid::now_v7())
    }
}

/// Credential provider backed by the process secret store.
///
/// The configured `device_password_ref` template may contain the `{device_id}`
/// placeholder, which is replaced with the GB28181 device identifier before the
/// secret store is queried. Missing optional secrets return `Ok(None)` so the
/// domain can fall back to challenge-based authentication when enabled; backend
/// failures are returned as `Err(CredentialError::Backend(...))`.
#[derive(Clone)]
struct SecretStoreCredentialProvider {
    store: Arc<dyn SecretStore>,
    ref_template: Option<String>,
}

impl std::fmt::Debug for SecretStoreCredentialProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStoreCredentialProvider")
            .field("ref_template", &self.ref_template)
            .finish_non_exhaustive()
    }
}

impl SecretStoreCredentialProvider {
    fn new(store: Arc<dyn SecretStore>, ref_template: Option<String>) -> Self {
        Self {
            store,
            ref_template,
        }
    }
}

impl CredentialProvider for SecretStoreCredentialProvider {
    fn password_for(
        &self,
        device_id: &GbDeviceId,
    ) -> Result<Option<SecretString>, CredentialError> {
        let Some(template) = self.ref_template.as_ref() else {
            return Ok(None);
        };
        let key = template.replace("{device_id}", device_id.as_ref());
        match self.store.get(&key) {
            Ok(secret) => Ok(Some(secret)),
            Err(e) if e.kind() == cheetah_signal_types::SignalErrorKind::NotFound => Ok(None),
            Err(e) => Err(CredentialError::Backend(e.to_string())),
        }
    }
}

/// Adapts a [`RawEventBus`] into a domain [`EventPublisher`] so it can be used
/// by the outbox relay and any application service that expects the domain trait.
#[derive(Clone)]
struct EventBusPublisher {
    bus: Arc<dyn RawEventBus>,
}

impl std::fmt::Debug for EventBusPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBusPublisher").finish_non_exhaustive()
    }
}

impl EventBusPublisher {
    fn new(bus: Arc<dyn RawEventBus>) -> Self {
        Self { bus }
    }
}

#[async_trait::async_trait]
impl EventPublisher for EventBusPublisher {
    async fn publish(&self, event: &Event<DomainEvent>) -> cheetah_domain::Result<()> {
        publish_domain_event(&*self.bus, event).await
    }
}

/// Builds the process secret store from the configured sources.
fn build_secret_store(config: &SignalConfig) -> Arc<dyn SecretStore> {
    let env_store = EnvSecretStore::with_prefix(&config.secret.env_prefix);
    let mut stores: Vec<Arc<dyn SecretStore>> = vec![Arc::new(env_store) as Arc<dyn SecretStore>];
    if let Some(dir) = config.secret.file_dir.as_deref() {
        stores.push(Arc::new(FileSecretStore::new(dir)) as Arc<dyn SecretStore>);
    }
    Arc::new(CompositeSecretStore::new(stores)) as Arc<dyn SecretStore>
}

/// Resolves a stable node identity.
///
/// If `system.node_id` is configured it takes precedence. Otherwise the id is
/// read from `<data_dir>/node_id`; when missing a new UUIDv7 is generated,
/// written to disk, and reused on subsequent starts.
fn resolve_node_id(
    config: &SignalConfig,
    id_generator: &dyn IdGenerator,
) -> Result<NodeId, Box<dyn std::error::Error + Send + Sync>> {
    if let Some(id) = config.system.node_id {
        return Ok(id);
    }

    if !config.system.data_dir.is_empty() {
        let mut path = PathBuf::from(&config.system.data_dir);
        path.push("node_id");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let text = text.trim();
            if !text.is_empty() {
                match text.parse::<NodeId>() {
                    Ok(id) => return Ok(id),
                    Err(e) => {
                        warn!(path = %path.display(), error = %e, "invalid persisted node_id");
                    }
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let id = id_generator.generate_node_id();
        std::fs::write(&path, id.to_string())?;
        info!(%id, path = %path.display(), "persisted generated node_id");
        return Ok(id);
    }

    let id = id_generator.generate_node_id();
    info!(%id, "generated transient node_id");
    Ok(id)
}

/// Resolves the GB28181 SIP digest secret from the secret store.
fn resolve_gb28181_digest_secret(
    secret_store: &dyn SecretStore,
    ref_key: &str,
) -> Result<SecretSlice<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let secret = secret_store
        .get(ref_key)
        .map_err(|e| format!("failed to resolve gb28181 digest secret ({ref_key}): {e}"))?;
    let hex_secret = secret.expose_secret();
    let bytes = hex::decode(hex_secret.trim())
        .map_err(|e| format!("gb28181 digest secret ({ref_key}) must be hex-encoded: {e}"))?;
    if bytes.len() < 32 {
        return Err(
            format!("gb28181 digest secret ({ref_key}) must decode to at least 32 bytes").into(),
        );
    }
    Ok(SecretSlice::from(bytes))
}

/// Assembles storage, local bus, application services, protocol drivers and the HTTP API.
pub async fn start(
    config: SignalConfig,
) -> Result<SignalingRuntime, Box<dyn std::error::Error + Send + Sync>> {
    config.validate()?;

    let secret_store = build_secret_store(&config);

    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let id_generator: Arc<dyn IdGenerator> = Arc::new(UuidIdGenerator);
    let cancel = CancellationToken::new();
    let mut workers = Vec::new();

    let node_id = resolve_node_id(&config, id_generator.as_ref())?;
    let node_instance_id = id_generator.generate_node_instance_id();
    info!(%node_id, %node_instance_id, "node identity ready");

    let storage: Arc<dyn Storage> = match config.storage.backend {
        StorageBackend::Sqlite => {
            let path = PathBuf::from(&config.storage.sqlite_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let sqlite = SqliteStorage::new(&path).await?;
            sqlite.migration().run().await?;
            info!(path = %path.display(), "sqlite storage ready");
            Arc::new(sqlite)
        }
        StorageBackend::Postgres => {
            let url = if let Some(ref_key) = config.storage.postgres_url_ref.as_deref() {
                secret_store
                    .get(ref_key)
                    .map_err(|e| {
                        format!("failed to resolve storage.postgres_url_ref ({ref_key}): {e}")
                    })?
                    .expose_secret()
                    .to_string()
            } else {
                config.storage.postgres_url.expose_secret().to_string()
            };
            if url.is_empty() {
                return Err(
                    "storage.postgres_url or storage.postgres_url_ref is required when backend=postgres".into(),
                );
            }
            let pg = cheetah_storage_postgres::PostgresStorage::new(&url).await?;
            pg.migration().run().await?;
            info!("postgres storage ready");
            Arc::new(pg)
        }
        _ => {
            return Err("unsupported storage.backend; use sqlite or postgres".into());
        }
    };

    let owner_repo: Arc<dyn cheetah_storage_api::OwnerRepository> =
        storage.owner_repository().into();
    let owner_resolver: Arc<dyn DeviceOwnerResolver> = Arc::new(CachingDeviceOwnerResolver::new(
        owner_repo,
        clock.clone(),
        DurationMs::from_millis(1000),
        1024,
    ));

    let bus: Arc<dyn RawEventBus> = match config.messaging.backend {
        MessagingBackend::Local => Arc::new(InProcessMessageBus::new(
            config.messaging.max_pending.max(64),
            config.messaging.max_pending.max(64),
        )),
        MessagingBackend::Nats => {
            let url = if let Some(ref_key) = config.messaging.nats_url_ref.as_deref() {
                secret_store
                    .get(ref_key)
                    .map_err(|e| {
                        format!("failed to resolve messaging.nats_url_ref ({ref_key}): {e}")
                    })?
                    .expose_secret()
                    .to_string()
            } else {
                config.messaging.nats_url.clone()
            };
            if url.is_empty() {
                return Err(
                    "messaging.nats_url or messaging.nats_url_ref is required when backend=nats"
                        .into(),
                );
            }
            let scheme = url.split("://").next().unwrap_or(&url).to_lowercase();
            if !matches!(scheme.as_str(), "tls" | "wss") {
                return Err(
                    "messaging.nats_url must use tls:// or wss:// scheme for cluster deployments"
                        .into(),
                );
            }
            let connect_timeout = Duration::from_secs(5);
            let operation_timeout = Duration::from_secs(30);
            Arc::new(
                NatsBus::connect(
                    url,
                    node_id,
                    owner_resolver.clone(),
                    connect_timeout,
                    operation_timeout,
                )
                .await?,
            ) as Arc<dyn RawEventBus>
        }
        _ => {
            return Err("unsupported messaging.backend; use local or nats".into());
        }
    };
    let publisher: Arc<dyn EventPublisher> = Arc::new(EventBusPublisher::new(bus.clone()));

    let media_registry = Arc::new(InMemoryMediaNodeRegistry::new(
        MediaRegistryConfig::default(),
    ));
    let media_scheduler: Arc<dyn cheetah_media_scheduler::MediaScheduler> = Arc::new(
        LeastLoadedScheduler::new(media_registry, SchedulerConfig::default()),
    );
    let media_client = MediaControlClient::new(MediaClientConfig::default())
        .with_secret_store(secret_store.clone());
    let media_port: Arc<dyn MediaPort> =
        Arc::new(SchedulerMediaPort::new(media_scheduler, media_client));

    // Outbox relay: publish pending domain events without holding DB transactions
    // across the message bus I/O boundary.
    {
        let relay = OutboxRelay::new(storage.clone(), publisher, clock.clone(), 64);
        let worker_cancel = cancel.child_token();
        workers.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = worker_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        match relay.run_once().await {
                            Ok(0) => {}
                            Ok(n) => info!(published = n, "outbox relay batch"),
                            Err(e) => warn!(error = %e, "outbox relay failed"),
                        }
                    }
                }
            }
        }));
        info!("outbox relay worker started");
    }

    // Shared application state is constructed before protocol adapters so that
    // inbound protocol events can be routed through application services.
    let api_config = ApiConfig::from(&config);
    let mut state = ApiState::new(
        api_config,
        storage.clone(),
        clock,
        id_generator,
        bus.clone(),
        owner_resolver,
        media_port,
    );
    state.cancel = cancel.clone();

    let default_tenant_id = config
        .gb28181
        .default_tenant_id
        .as_ref()
        .map(|s| s.parse::<TenantId>())
        .transpose()
        .map_err(|e| format!("gb28181.default_tenant_id is not a valid UUID: {e}"))?;

    // GB28181 UDP access listener.
    let mut gb28181_addr = None;
    if config.gb28181.sip_port > 0 {
        let domain_id = if config.gb28181.sip_domain.is_empty() {
            "34020000002000000001".to_string()
        } else {
            config.gb28181.sip_domain.clone()
        };
        let realm = domain_id.clone();
        let digest_ref = config
            .gb28181
            .digest_secret_ref
            .as_deref()
            .ok_or("gb28181.digest_secret_ref is required when sip_port > 0")?;
        let digest_secret = resolve_gb28181_digest_secret(&*secret_store, digest_ref)?;
        let domain_config = Gb28181DomainConfig::new(&domain_id, &realm, digest_secret)
            .map_err(|e| format!("gb28181 domain config: {e}"))?
            .with_auth_policy(AuthPolicy::ChallengeOptional);

        let credential_provider = SecretStoreCredentialProvider::new(
            secret_store.clone(),
            config.gb28181.device_password_ref.clone(),
        );

        let bind = SocketAddr::from(([0, 0, 0, 0], config.gb28181.sip_port));
        let driver_config = GbDriverConfig::new(bind);
        let (sink, gb_event_handle) = gb_event_sink::spawn(
            state.clone(),
            node_id,
            default_tenant_id,
            config.runtime.queue_depth,
            config.gb28181.catalog_fragment_max_entries as usize,
            config.gb28181.catalog_fragment_max_items as usize,
            cancel.child_token(),
        );
        workers.push(gb_event_handle);
        let (driver, local) =
            Gb28181UdpDriver::bind(driver_config, domain_config, credential_provider, sink)
                .await
                .map_err(|e| format!("gb28181 bind failed: {e}"))?;
        gb28181_addr = Some(local);
        let worker_cancel = cancel.child_token();
        workers.push(tokio::spawn(async move {
            tokio::select! {
                _ = worker_cancel.cancelled() => {
                    info!("gb28181 driver cancelled");
                }
                result = driver.run() => {
                    if let Err(e) = result {
                        warn!(error = %e, "gb28181 driver exited with error");
                    }
                }
            }
        }));
        info!(%local, "gb28181 SIP UDP listening");
    } else {
        warn!("gb28181.sip_port is 0; protocol UDP listener not started");
    }

    let http = ApiServer::start(state).await?;
    let http_addr = http.local_addr;
    info!(%http_addr, "HTTP API listening");

    Ok(SignalingRuntime {
        cancel,
        http,
        http_addr,
        gb28181_addr,
        workers,
    })
}
