//! Process assembly for edge and cluster profiles.
//!
//! Startup order (per AGENTS.md): config/secret → schema check → bus →
//! repository → ownership → media → protocol → public listener → ready.

use crate::gb_event_sink;
use crate::onvif_discovery;
use crate::workers::{
    DriverCommandBus, OwnerCommandHandler, SingleNodeOwnerResolver, StorageDeviceProtocolLookup,
    build_assignment_service, build_drain_service, build_takeover_service, builtin_plugin_ids,
    spawn_drain_migration_worker, spawn_inbox_worker, spawn_node_lease_worker,
    spawn_owner_lease_renew_worker, spawn_protocol_session_reaper_worker,
    spawn_takeover_health_worker,
};
use ::time::{OffsetDateTime, UtcOffset};
use cheetah_cluster_ownership::{CachingDeviceOwnerResolver, OwnerLeaseService};
use cheetah_cluster_registry::NodeLeaseService;
use cheetah_domain::ports::{DeviceOwnerResolver, MediaPort};
use cheetah_domain::{DomainEvent, EventPublisher, MediaEventHandler};
use cheetah_gb28181_core::{
    BranchPolicy, BroadcastAddressSource, BroadcastOverride, CompatibilityCapability,
    CompatibilityOverrides, CompatibilityProfile, ManagerConfig, MediaStatusOverride,
    SdpMediaOverride,
};
use cheetah_gb28181_driver_tokio::Gb28181UdpDriver;
use cheetah_gb28181_driver_tokio::config::DriverConfig as GbDriverConfig;
use cheetah_gb28181_module::{GbAccessSettings, build_access};
use cheetah_http_api::audit::TracingAuditLog;
use cheetah_http_api::state::{ApiConfig, ApiServer, ApiState};
use cheetah_media_client::{MediaClientConfig, MediaControlClient};
use cheetah_media_scheduler::{
    LeastLoadedScheduler, MediaClusterRegistryService, MediaEventConsumer,
    MediaEventConsumerConfig, MediaRegistryConfig, NoopReconciliationHandler, PeerIdentity,
    PersistentMediaNodeRegistry, SchedulerConfig, SchedulerMediaPort,
};
use cheetah_message_api::publisher::publish_domain_event;
use cheetah_message_api::{RawCommandBus, RawEventBus};
use cheetah_message_local::InProcessMessageBus;
use cheetah_message_nats::NatsBus;
use cheetah_onvif_driver_tokio::OnvifTokioDriverFactory;
use cheetah_plugin_host::PluginHost;
use cheetah_plugin_sdk::{PluginManifest, ProtocolDriverFactory};
use cheetah_secret::{CompositeSecretStore, EnvSecretStore, FileSecretStore};
use cheetah_signal_application::OutboxRelay;
use cheetah_signal_contracts::cheetah::common::v1::media_cluster_registry_server::MediaClusterRegistryServer;
use cheetah_signal_types::config::{
    Gb28181CompatibilityOverridesConfig, Gb28181CompatibilityProfileConfig,
};
use cheetah_signal_types::config::{MessagingBackend, SignalConfig, StorageBackend};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, DurationMs, Event, IdGenerator, MediaBindingId, MediaSessionId,
    NodeId, SecretStore, TenantId, UtcTimestamp,
};
use cheetah_storage_api::Storage;
use cheetah_storage_sqlite::SqliteStorage;
use futures::future::select_all;
use secrecy::{ExposeSecret, SecretString};
use semver::Version;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tonic::service::InterceptorLayer;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig, server::TcpIncoming};
use tracing::{info, warn};
use uuid::Uuid;
use x509_parser::prelude::{FromDer, X509Certificate};

/// Health status of a single runtime component.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ComponentStatus {
    /// Component is running.
    Running,
    /// Component stopped cleanly.
    Stopped,
    /// Component stopped with an error.
    Failed(String),
}

/// Aggregated health of all supervised runtime components.
#[derive(Clone, Debug, Default)]
pub struct RuntimeHealth {
    /// Component name to last observed status.
    pub components: HashMap<String, ComponentStatus>,
}

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
    /// Bound internal gRPC address.
    pub grpc_addr: SocketAddr,
    /// Plugin host with built-in factories and validated external manifests.
    #[allow(dead_code)]
    pub plugin_host: Arc<tokio::sync::Mutex<PluginHost>>,
    /// Readiness flag: true only after all startup stages have completed.
    pub ready: Arc<AtomicBool>,
    /// Observed health of supervised background components.
    pub health: Arc<Mutex<RuntimeHealth>>,
    /// Background worker handles (outbox, protocol drivers).
    workers: Vec<JoinHandle<()>>,
}

impl SignalingRuntime {
    /// Gracefully shuts down the runtime.
    ///
    /// 1. Marks readiness false.
    /// 2. Cancels all worker cancellation tokens.
    /// 3. Waits for workers to finish within `worker_timeout`.
    /// 4. Aborts any remaining workers and stops the HTTP server.
    pub async fn shutdown(self, worker_timeout: Duration) -> RuntimeHealth {
        self.ready.store(false, Ordering::SeqCst);
        self.cancel.cancel();

        let mut health = self
            .health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut remaining: Vec<(usize, JoinHandle<()>)> =
            self.workers.into_iter().enumerate().collect();
        let deadline = tokio::time::Instant::now() + worker_timeout;

        while !remaining.is_empty() {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() {
                break;
            }

            let handles: Vec<&mut JoinHandle<()>> = remaining.iter_mut().map(|(_, h)| h).collect();
            let (result, idx, _) = match timeout(left, select_all(handles)).await {
                Ok(r) => r,
                Err(_) => break,
            };

            let (original_idx, handle) = remaining.remove(idx);
            drop(handle);
            let name = format!("worker-{original_idx}");
            match result {
                Ok(()) => {
                    health.components.insert(name, ComponentStatus::Stopped);
                }
                Err(e) => {
                    let msg = format!("{e}");
                    warn!(component = %name, error = %msg, "worker exited with error");
                    health.components.insert(name, ComponentStatus::Failed(msg));
                }
            }
        }

        for (i, handle) in remaining {
            handle.abort();
            health.components.insert(
                format!("worker-{i}"),
                ComponentStatus::Failed("shutdown deadline exceeded; worker aborted".to_string()),
            );
        }

        self.http.shutdown();
        health
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

/// Plugin secret provider backed by the process secret store.
struct ProcessSecretProvider {
    store: Arc<dyn SecretStore>,
}

#[async_trait::async_trait]
impl cheetah_plugin_host::SecretProvider for ProcessSecretProvider {
    async fn get_secret(
        &self,
        name: &str,
    ) -> Result<Option<SecretString>, cheetah_plugin_sdk::PluginError> {
        match self.store.get(name) {
            Ok(secret) => Ok(Some(secret)),
            Err(e) if e.kind() == cheetah_signal_types::SignalErrorKind::NotFound => Ok(None),
            Err(e) => Err(cheetah_plugin_sdk::PluginError::Driver(e.to_string())),
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

/// Owner repository that mints a fresh pool-backed handle per call.
struct StorageBackedOwnerRepo {
    storage: Arc<dyn Storage>,
}

#[async_trait::async_trait]
impl cheetah_storage_api::OwnerRepository for StorageBackedOwnerRepo {
    async fn get(
        &self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<Option<cheetah_domain::OwnerInfo>, cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .get(tenant_id, device_id)
            .await
    }

    async fn set(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        owner: cheetah_domain::OwnerInfo,
    ) -> Result<(), cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .set(tenant_id, device_id, owner)
            .await
    }

    async fn clear(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
    ) -> Result<(), cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .clear(tenant_id, device_id)
            .await
    }

    async fn acquire(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        now: UtcTimestamp,
        lease_until: UtcTimestamp,
    ) -> Result<cheetah_domain::OwnerInfo, cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .acquire(tenant_id, device_id, node_id, now, lease_until)
            .await
    }

    async fn renew(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        lease_until: UtcTimestamp,
    ) -> Result<Option<cheetah_domain::OwnerInfo>, cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .renew(tenant_id, device_id, node_id, lease_until)
            .await
    }

    async fn release(
        &mut self,
        tenant_id: TenantId,
        device_id: DeviceId,
        node_id: NodeId,
        epoch: cheetah_signal_types::OwnerEpoch,
    ) -> Result<(), cheetah_storage_api::StorageError> {
        self.storage
            .owner_repository()
            .release(tenant_id, device_id, node_id, epoch)
            .await
    }

    async fn list_by_node(
        &self,
        node_id: NodeId,
        page: cheetah_signal_types::PageRequest,
    ) -> Result<
        cheetah_signal_types::Page<cheetah_storage_api::OwnedDevice>,
        cheetah_storage_api::StorageError,
    > {
        self.storage
            .owner_repository()
            .list_by_node(node_id, page)
            .await
    }
}

/// Node repository that mints a fresh pool-backed handle per call.
struct StorageBackedNodeRepo {
    storage: Arc<dyn Storage>,
}

#[async_trait::async_trait]
impl cheetah_storage_api::NodeRepository for StorageBackedNodeRepo {
    async fn register(
        &mut self,
        node: cheetah_domain::ClusterNode,
    ) -> Result<(), cheetah_storage_api::StorageError> {
        self.storage.node_repository().register(node).await
    }

    async fn heartbeat(
        &mut self,
        node_id: NodeId,
        instance_id: cheetah_signal_types::NodeInstanceId,
        lease_until: UtcTimestamp,
        updated_at: UtcTimestamp,
        load: cheetah_domain::NodeLoad,
    ) -> Result<Option<cheetah_domain::ClusterNode>, cheetah_storage_api::StorageError> {
        self.storage
            .node_repository()
            .heartbeat(node_id, instance_id, lease_until, updated_at, load)
            .await
    }

    async fn get(
        &self,
        node_id: NodeId,
    ) -> Result<Option<cheetah_domain::ClusterNode>, cheetah_storage_api::StorageError> {
        self.storage.node_repository().get(node_id).await
    }

    async fn list_alive(
        &self,
        now: UtcTimestamp,
        page: cheetah_signal_types::PageRequest,
    ) -> Result<
        cheetah_signal_types::Page<cheetah_domain::ClusterNode>,
        cheetah_storage_api::StorageError,
    > {
        self.storage.node_repository().list_alive(now, page).await
    }

    async fn mark_draining(
        &mut self,
        node_id: NodeId,
        instance_id: cheetah_signal_types::NodeInstanceId,
        updated_at: UtcTimestamp,
    ) -> Result<bool, cheetah_storage_api::StorageError> {
        self.storage
            .node_repository()
            .mark_draining(node_id, instance_id, updated_at)
            .await
    }
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

    let owner_repo_arc: Arc<dyn cheetah_storage_api::OwnerRepository> =
        Arc::new(StorageBackedOwnerRepo {
            storage: storage.clone(),
        });
    let owner_repo_mutex: Arc<tokio::sync::Mutex<dyn cheetah_storage_api::OwnerRepository>> =
        Arc::new(tokio::sync::Mutex::new(StorageBackedOwnerRepo {
            storage: storage.clone(),
        }));

    let lease_ttl = if config.cluster.lease_ttl_ms.as_millis() > 0 {
        config.cluster.lease_ttl_ms
    } else {
        DurationMs::from_millis(10_000)
    };
    let owner_lease = Arc::new(OwnerLeaseService::new(
        owner_repo_mutex.clone(),
        clock.clone(),
        node_id,
        lease_ttl,
    ));

    let caching_resolver = CachingDeviceOwnerResolver::new(
        owner_repo_arc.clone(),
        clock.clone(),
        DurationMs::from_millis(1000),
        1024,
    );
    // Edge/single-node: auto-acquire ownership when missing. Cluster still uses
    // the caching resolver; assignment workers fill ownership separately.
    let owner_resolver: Arc<dyn DeviceOwnerResolver> = if config.cluster.enabled {
        Arc::new(caching_resolver)
    } else {
        Arc::new(SingleNodeOwnerResolver::new(
            OwnerLeaseService::new(owner_repo_mutex.clone(), clock.clone(), node_id, lease_ttl),
            caching_resolver,
        ))
    };

    let (bus, command_bus): (Arc<dyn RawEventBus>, Arc<dyn RawCommandBus>) =
        match config.messaging.backend {
            MessagingBackend::Local => {
                let local = Arc::new(InProcessMessageBus::new(
                    config.messaging.max_pending.max(64),
                    config.messaging.max_pending.max(64),
                ));
                (local.clone(), local)
            }
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
                let nats = Arc::new(
                    NatsBus::connect(
                        url,
                        node_id,
                        owner_resolver.clone(),
                        connect_timeout,
                        operation_timeout,
                    )
                    .await?,
                );
                (nats.clone(), nats)
            }
            _ => {
                return Err("unsupported messaging.backend; use local or nats".into());
            }
        };
    let publisher: Arc<dyn EventPublisher> = Arc::new(EventBusPublisher::new(bus.clone()));

    let mut media_registry_config = MediaRegistryConfig::production();
    let media_metrics = cheetah_media_scheduler::MediaMetrics::arc();
    let media_repo = storage.media_node_repository();
    let persistent_registry =
        PersistentMediaNodeRegistry::new(media_registry_config.clone(), media_repo);
    persistent_registry
        .load(clock.as_ref())
        .await
        .map_err(|e| format!("failed to load media node registry: {e}"))?;
    let media_registry: Arc<dyn cheetah_media_scheduler::MediaNodeRegistry> =
        Arc::new(persistent_registry);
    let media_registry_for_grpc = Arc::clone(&media_registry);
    let media_registry_for_consumer = Arc::clone(&media_registry);
    let media_scheduler: Arc<dyn cheetah_media_scheduler::MediaScheduler> = Arc::new(
        LeastLoadedScheduler::new(media_registry, SchedulerConfig::default()),
    );
    let media_client = MediaControlClient::new(MediaClientConfig::default())
        .with_secret_store(secret_store.clone());
    let media_client_for_consumer = media_client.clone();
    let media_port: Arc<dyn MediaPort> = Arc::new(SchedulerMediaPort::new(
        media_scheduler,
        media_client,
        media_metrics.clone(),
    ));

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

    // Placeholder; plugin_host is constructed below and shared with the command handler.
    // Inbox is started after plugin activation so handle_command can reach real drivers.

    // Owner lease renew for devices currently owned by this node.
    {
        let renew_interval = Duration::from_millis(
            u64::try_from(lease_ttl.as_millis().max(1_000) / 3).unwrap_or(3_000),
        );
        workers.push(spawn_owner_lease_renew_worker(
            owner_lease.clone(),
            owner_repo_arc.clone(),
            node_id,
            renew_interval,
            cancel.child_token(),
        ));
        info!(?renew_interval, "owner lease renew worker started");
    }

    // Cluster/edge node registration + heartbeat against cluster_nodes table.
    {
        let node_repo: Arc<tokio::sync::Mutex<dyn cheetah_storage_api::NodeRepository>> =
            Arc::new(tokio::sync::Mutex::new(StorageBackedNodeRepo {
                storage: storage.clone(),
            }));
        let heartbeat_ms = if config.cluster.heartbeat_interval_ms.as_millis() > 0 {
            config.cluster.heartbeat_interval_ms.as_millis()
        } else {
            3_000
        };
        let node_lease = NodeLeaseService::new(
            node_repo,
            clock.clone(),
            id_generator.clone(),
            node_id,
            config.system.node_name.clone(),
            env!("CARGO_PKG_VERSION"),
            lease_ttl,
        );
        workers.push(spawn_node_lease_worker(
            node_lease,
            Duration::from_millis(u64::try_from(heartbeat_ms).unwrap_or(3_000)),
            cancel.child_token(),
        ));
        info!("node lease/heartbeat worker started");
    }

    // Shared application state is constructed before protocol adapters so that
    // inbound protocol events can be routed through application services.
    let audit: Arc<dyn cheetah_signal_types::AuditLog> = Arc::new(TracingAuditLog);
    let api_config = ApiConfig::from(&config);
    // GB28181 runtime/application metrics aggregator. It is exposed on the
    // `/metrics` endpoint and drives runtime readiness/degraded reporting on
    // `/readyz`. Series cardinality is fixed by the shard count and the bounded
    // label enums, so no tenant/device identifiers reach Prometheus.
    let gb_metrics = Arc::new(cheetah_runtime_tokio::GbMetrics::new(
        config.runtime.worker_threads.max(1),
        config.runtime.queue_depth as u64,
    ));
    let mut state = ApiState::new(
        api_config,
        storage.clone(),
        clock.clone(),
        id_generator.clone(),
        bus.clone(),
        owner_resolver.clone(),
        media_port,
    )
    .with_media_metrics(media_metrics.clone())
    .with_gb_metrics(gb_metrics.clone())
    .with_runtime_health(gb_metrics.clone())
    .with_audit(audit.clone());
    state.cancel = cancel.clone();

    // Media event consumer: subscribe to active media nodes and apply
    // session-level callbacks through the application media service.
    let media_event_handler: Arc<dyn MediaEventHandler> = Arc::new(state.media_service.clone());
    let media_event_consumer = Arc::new(MediaEventConsumer::new(
        media_registry_for_consumer,
        media_client_for_consumer,
        media_event_handler,
        storage.clone(),
        clock.clone(),
        node_id,
        MediaEventConsumerConfig::default(),
        Arc::new(NoopReconciliationHandler),
        media_metrics.clone(),
    ));
    let consumer_cancel = cancel.child_token();
    workers.push(tokio::spawn(async move {
        if let Err(e) = media_event_consumer.run(consumer_cancel).await {
            warn!(error = %e, "media event consumer exited");
        }
    }));
    info!("media event consumer worker started");

    // Plugin host: register built-in GB28181 and ONVIF factories, activate them,
    // and validate any external plugin manifests before protocol workers start.
    let host_sdk_version = Version::new(0, 1, 0);
    let secret_provider: Arc<dyn cheetah_plugin_host::SecretProvider> =
        Arc::new(ProcessSecretProvider {
            store: secret_store.clone(),
        });
    let mut plugin_host = PluginHost::with_secret_provider(
        host_sdk_version,
        config.onvif.request_timeout_ms,
        secret_provider,
    );
    let onvif_factory = OnvifTokioDriverFactory::new();
    let onvif_name = onvif_factory.name();
    plugin_host
        .register_builtin(onvif_name.clone(), Box::new(onvif_factory))
        .map_err(|e| format!("failed to register onvif plugin factory: {e}"))?;

    let onvif_plugin_id = builtin_plugin_ids(id_generator.as_ref()).1;
    // Activate ONVIF first (start is lightweight).
    if let Err(e) = plugin_host
        .activate_builtin(onvif_plugin_id, onvif_name, serde_json::json!({}), None)
        .await
    {
        warn!(error = %e, "failed to activate built-in ONVIF plugin instance");
    } else {
        info!(%onvif_plugin_id, "activated built-in ONVIF plugin instance");
    }

    if config.plugins.enabled {
        validate_external_plugins(
            &plugin_host,
            &config.plugins.plugin_dir,
            config.plugins.max_plugin_instances,
        )
        .await?;
    }

    let plugin_host = Arc::new(tokio::sync::Mutex::new(plugin_host));

    // Inbox consumer is spawned after GB28181 driver bind so the owner's command
    // handler can inject outbound commands into the driver.

    // Cluster drain/migration + takeover service (armed for reconnect paths).
    if config.cluster.enabled {
        let assignment = build_assignment_service(
            Arc::new(tokio::sync::Mutex::new(StorageBackedNodeRepo {
                storage: storage.clone(),
            })),
            owner_repo_mutex.clone(),
            clock.clone(),
            lease_ttl,
        );
        let drain_service = Arc::new(build_drain_service(
            assignment,
            owner_repo_mutex.clone(),
            Arc::new(tokio::sync::Mutex::new(StorageBackedNodeRepo {
                storage: storage.clone(),
            })),
            clock.clone(),
        ));
        let protocol_lookup: Arc<dyn cheetah_cluster_ownership::DeviceProtocolLookup> =
            Arc::new(StorageDeviceProtocolLookup::new(storage.clone()));
        workers.push(spawn_drain_migration_worker(
            drain_service,
            Arc::new(StorageBackedNodeRepo {
                storage: storage.clone(),
            }),
            protocol_lookup,
            clock.clone(),
            node_id,
            Duration::from_secs(30),
            cancel.child_token(),
        ));
        info!("drain migration worker started");
    }

    {
        let takeover = Arc::new(build_takeover_service(
            storage.clone(),
            clock.clone(),
            id_generator.clone(),
            node_id,
            lease_ttl,
        ));
        workers.push(spawn_takeover_health_worker(
            takeover,
            Duration::from_secs(60),
            cancel.child_token(),
        ));
        info!("takeover service armed");
    }

    // GB28181 access listeners. Each listener binds its own sockets and maps to
    // an explicit realm/domain/tenant. All protocol business mapping (domain
    // defaults, auth policy, digest secret resolution, credential provider)
    // lives in the module's assembly adapter; here we only inject dependencies
    // and manage the driver lifecycle.
    //
    // Legacy single-listener settings (`sip_port`/`sip_domain`/
    // `default_tenant_id`/...) are converted into a single synthetic listener
    // during a compatibility window; validation rejects mixing them with the
    // explicit `listeners` list.
    let (gb_listeners, gb_legacy) = config.gb28181.resolve_listeners();
    if gb_legacy {
        warn!(
            "gb28181 legacy sip_port/sip_domain/default_tenant_id settings are deprecated; \
             migrate to gb28181.listeners. Converting to a single listener for now."
        );
    }
    let mut gb28181_addr = None;
    let mut gb_command_tx: Option<tokio::sync::mpsc::Sender<_>> = None;
    if gb_listeners.is_empty() {
        warn!("no gb28181 listeners configured; protocol listener not started");
    }
    for listener in &gb_listeners {
        if listener.challenge_optional {
            warn!(
                listener_id = %listener.id,
                "gb28181 listener challenge_optional is enabled; unauthenticated REGISTER is accepted (dev profile only)"
            );
        }

        // An empty tenant is only reachable through the legacy compatibility
        // path (no default_tenant_id); explicit listeners require a tenant.
        // When absent, unattributable events are dropped by the sink.
        let tenant_id = if listener.tenant_id.is_empty() {
            None
        } else {
            Some(listener.tenant_id.parse::<TenantId>().map_err(|e| {
                format!(
                    "gb28181 listener '{}' tenant_id is not a valid UUID: {e}",
                    listener.id
                )
            })?)
        };

        let compatibility_profile = listener
            .compatibility_profile
            .as_deref()
            .map(|id| build_compatibility_profile(&config.gb28181.compatibility_profiles, id))
            .transpose()
            .map_err(|e| {
                format!(
                    "gb28181 listener '{}' compatibility profile: {e}",
                    listener.id
                )
            })?;

        let settings = GbAccessSettings::new(
            &listener.local_device_id,
            listener.digest_secret_ref.clone(),
        )
        .with_realm(&listener.realm)
        .with_challenge_optional(listener.challenge_optional)
        .with_device_password_ref(listener.device_password_ref.clone())
        .with_compatibility_profile(compatibility_profile.clone());
        let access = build_access(&settings, &secret_store).map_err(|e| {
            format!(
                "gb28181 listener '{}' access assembly failed: {e}",
                listener.id
            )
        })?;

        let branch_policy = compatibility_profile
            .as_ref()
            .map_or(BranchPolicy::Strict, |p| {
                if p.has(CompatibilityCapability::HeaderNormalization) {
                    BranchPolicy::Permissive
                } else {
                    BranchPolicy::Strict
                }
            });

        let mut driver_config = GbDriverConfig::empty()
            .with_manager_config(ManagerConfig {
                branch_policy,
                ..ManagerConfig::default()
            })
            .with_compatibility_profile(compatibility_profile);
        if let Some(udp) = listener.udp_bind {
            driver_config = driver_config.with_udp_bind(udp);
        }
        if let Some(tcp) = listener.tcp_bind {
            driver_config = driver_config.with_tcp_bind(tcp);
        }

        let (sink, gb_event_handle) = gb_event_sink::spawn(
            state.clone(),
            node_id,
            tenant_id,
            config.runtime.queue_depth,
            config.gb28181.catalog_fragment_max_entries as usize,
            config.gb28181.catalog_fragment_max_items as usize,
            config.gb28181.record_fragment_max_entries as usize,
            config.gb28181.record_fragment_max_items as usize,
            gb_metrics.clone(),
            cancel.child_token(),
        );
        workers.push(gb_event_handle);

        let (driver, local) = Gb28181UdpDriver::bind(driver_config, access, sink)
            .await
            .map_err(|e| format!("gb28181 listener '{}' bind failed: {e}", listener.id))?;
        if gb28181_addr.is_none() {
            gb28181_addr = Some(local);
        }
        // Outbound commands are transmitted over the driver's first UDP socket
        // (see `run_with_cancellation` in cheetah-gb28181-driver-tokio). Only
        // adopt the command bus from a UDP-capable listener so commands are
        // never accepted against a driver that cannot transmit them. When no
        // listener has a UDP bind the bus stays unset and the command handler
        // rejects commands rather than silently dropping them.
        if gb_command_tx.is_none() && listener.udp_bind.is_some() {
            gb_command_tx = Some(driver.command_bus());
        }
        let worker_cancel = cancel.child_token();
        let listener_id = listener.id.clone();
        workers.push(tokio::spawn(async move {
            // The driver observes cancellation directly and performs a bounded
            // drain of in-flight connections before returning.
            if let Err(e) = driver.run_with_cancellation(worker_cancel).await {
                warn!(listener_id = %listener_id, error = %e, "gb28181 driver exited with error");
            } else {
                info!(listener_id = %listener_id, "gb28181 driver stopped");
            }
        }));
        info!(listener_id = %listener.id, %local, realm = %listener.realm, domain = %listener.domain, "gb28181 SIP listening");
    }

    // Inbox consumer after GB28181 driver bind so the command bus is wired.
    {
        if gb_command_tx.is_none() && !gb_listeners.is_empty() {
            warn!(
                "no gb28181 listener has a UDP bind; outbound device commands are disabled and \
                 will be rejected instead of silently dropped"
            );
        }
        let gb_bus = gb_command_tx.map(|tx| {
            Arc::new(DriverCommandBus::new(tx)) as Arc<dyn crate::workers::Gb28181CommandBus>
        });
        let handler: Arc<dyn cheetah_signal_application::CommandHandler> = Arc::new(
            OwnerCommandHandler::new(plugin_host.clone(), clock.clone(), gb_bus),
        );
        workers.push(spawn_inbox_worker(
            storage.clone(),
            command_bus.clone(),
            owner_resolver.clone(),
            handler,
            clock.clone(),
            node_id,
            cancel.child_token(),
        ));
        info!("inbox consumer worker started");
    }

    // Protocol session expiry reaper is a single global, cross-tenant worker.
    // Spawn it once after all listeners are configured, not once per listener.
    if !gb_listeners.is_empty() {
        let reaper_interval_ms = config.gb28181.session_reaper_interval_ms.as_millis();
        if reaper_interval_ms > 0 {
            let interval =
                Duration::from_millis(u64::try_from(reaper_interval_ms).unwrap_or(30_000));
            workers.push(spawn_protocol_session_reaper_worker(
                state.storage.clone(),
                state.clock.clone(),
                state.id_generator.clone(),
                interval,
                config.gb28181.session_reaper_batch_size.max(1),
                config.gb28181.session_reaper_max_per_tick.max(1) as usize,
                cancel.child_token(),
            ));
            info!("gb28181 protocol session reaper worker started");
        } else {
            warn!("gb28181.session_reaper_interval_ms is 0; expiry reaper not started");
        }
    }

    // ONVIF WS-Discovery worker: periodically probes the network for cameras
    // and provisions them through the application DeviceService.
    if config.onvif.enabled {
        workers.push(onvif_discovery::spawn(
            state.clone(),
            node_id,
            config.onvif.clone(),
            cancel.child_token(),
        ));
        info!("onvif discovery worker started");
    } else {
        warn!("onvif.enabled is false; discovery worker not started");
    }

    // Internal gRPC server for media node lifecycle (MediaClusterRegistry).
    // Binds the port eagerly so startup fails fast; TLS/mTLS is configured
    // when the corresponding secret references are provided. The router and its
    // TLS config are built synchronously so any misconfiguration fails startup
    // before readiness is advertised.
    let grpc_ip = config
        .grpc
        .listen_addr
        .parse::<IpAddr>()
        .map_err(|e| format!("grpc.listen_addr is not a valid IP address: {e}"))?;
    let grpc_addr = SocketAddr::new(grpc_ip, config.grpc.port);
    let (grpc_identity, grpc_client_ca, grpc_require_mtls) =
        configure_grpc_tls(&*secret_store, &config.grpc)?;
    media_registry_config.require_mtls = grpc_require_mtls;
    let grpc_service = MediaClusterRegistryService::new(
        media_registry_for_grpc,
        clock.clone(),
        id_generator.clone(),
        media_registry_config,
        media_metrics,
        audit,
        node_id,
    );
    let tcp_incoming = TcpIncoming::bind(grpc_addr)
        .map_err(|e| format!("failed to bind internal gRPC listener: {e}"))?;
    let grpc_addr = tcp_incoming
        .local_addr()
        .map_err(|e| format!("failed to get gRPC local address: {e}"))?;

    let mut server = Server::builder();
    if let Some(identity) = grpc_identity {
        let mut tls = ServerTlsConfig::new().identity(identity);
        if let Some(client_ca) = grpc_client_ca {
            tls = tls.client_ca_root(client_ca).client_auth_optional(false);
        }
        server = server
            .tls_config(tls)
            .map_err(|e| format!("failed to configure gRPC TLS: {e}"))?;
    }
    let grpc_router = server
        .layer(InterceptorLayer::new(mtls_identity_interceptor))
        .add_service(MediaClusterRegistryServer::new(grpc_service));
    let grpc_server = grpc_router
        .serve_with_incoming_shutdown(tcp_incoming, cancel.child_token().cancelled_owned());
    workers.push(tokio::spawn(async move {
        if let Err(e) = grpc_server.await {
            warn!(error = %e, "internal gRPC server stopped with error");
        }
    }));
    info!(%grpc_addr, "internal gRPC listening");

    let http = ApiServer::start(state).await?;
    let http_addr = http.local_addr;
    info!(%http_addr, "HTTP API listening");

    let ready = Arc::new(AtomicBool::new(false));
    let health = Arc::new(Mutex::new(RuntimeHealth::default()));
    ready.store(true, Ordering::SeqCst);
    info!("cheetah-signaling ready");

    Ok(SignalingRuntime {
        cancel,
        http,
        http_addr,
        gb28181_addr,
        grpc_addr,
        plugin_host,
        ready,
        health,
        workers,
    })
}

/// Validates external plugin manifests in `plugin_dir` without activating them.
///
/// Each `.json` file is parsed as a [`PluginManifest`] and checked against the
/// host SDK version and checksum by the plugin host. The number of validated
/// manifests is bounded by `max_instances`.
async fn validate_external_plugins(
    plugin_host: &PluginHost,
    plugin_dir: &str,
    max_instances: u32,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if plugin_dir.is_empty() {
        warn!(
            "plugins.enabled is true but plugins.plugin_dir is empty; skipping external plugin validation"
        );
        return Ok(());
    }

    let dir = PathBuf::from(plugin_dir);
    if !dir.is_dir() {
        warn!(path = %dir.display(), "plugins.plugin_dir does not exist; skipping external plugin validation");
        return Ok(());
    }

    let mut validated = 0u32;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let payload = std::fs::read(&path)?;
        let manifest: PluginManifest = match serde_json::from_slice(&payload) {
            Ok(m) => m,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "skipping invalid external plugin manifest");
                continue;
            }
        };

        match plugin_host.validate_manifest(&manifest, &payload) {
            Ok(validated_manifest) => {
                validated += 1;
                info!(
                    plugin = %validated_manifest.manifest.name,
                    version = %validated_manifest.plugin_version,
                    sdk = %validated_manifest.sdk_version,
                    "external plugin manifest validated"
                );
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "external plugin manifest validation failed");
            }
        }

        if validated > max_instances {
            return Err(format!(
                "external plugin manifest count exceeds plugins.max_plugin_instances ({max_instances})"
            )
            .into());
        }
    }

    if validated == 0 {
        warn!(path = %dir.display(), "no external plugin manifests validated");
    } else {
        info!(validated, "external plugin manifests validated");
    }

    Ok(())
}

/// Builds TLS identity and optional client CA from secret references.
///
/// Returns `(server_identity, client_ca, require_mtls)`. `require_mtls` is only
/// true when a client CA is configured, which forces the registry to require a
/// peer certificate identity matching the claimed node id.
fn configure_grpc_tls(
    secret_store: &dyn SecretStore,
    grpc: &cheetah_signal_types::config::GrpcConfig,
) -> Result<(Option<Identity>, Option<Certificate>, bool), String> {
    let Some(cert_ref) = grpc.tls_cert_ref.as_ref() else {
        if grpc.mtls_client_ca_ref.is_some() {
            return Err("grpc.mtls_client_ca_ref requires grpc.tls_cert_ref".to_string());
        }
        return Ok((None, None, false));
    };
    let Some(key_ref) = grpc.tls_key_ref.as_ref() else {
        return Err("grpc.tls_key_ref is required when grpc.tls_cert_ref is set".to_string());
    };
    let cert_pem = secret_store
        .get(cert_ref)
        .map_err(|e| format!("failed to load grpc.tls_cert_ref: {e}"))?
        .expose_secret()
        .to_string();
    let key_pem = secret_store
        .get(key_ref)
        .map_err(|e| format!("failed to load grpc.tls_key_ref: {e}"))?
        .expose_secret()
        .to_string();
    let identity = Identity::from_pem(cert_pem, key_pem);

    let mut client_ca: Option<Certificate> = None;
    if let Some(ca_ref) = grpc.mtls_client_ca_ref.as_ref() {
        let ca_pem = secret_store
            .get(ca_ref)
            .map_err(|e| format!("failed to load grpc.mtls_client_ca_ref: {e}"))?
            .expose_secret()
            .to_string();
        client_ca = Some(Certificate::from_pem(ca_pem));
    }

    Ok((Some(identity), client_ca, grpc.mtls_client_ca_ref.is_some()))
}

/// gRPC interceptor that extracts the mTLS peer common name and inserts it as a
/// [`PeerIdentity`] request extension so the registry can match the caller to a
/// media node id.
#[allow(clippy::unnecessary_wraps)]
fn mtls_identity_interceptor(
    mut req: tonic::Request<()>,
) -> Result<tonic::Request<()>, tonic::Status> {
    if let Some(tls_info) = req
        .extensions()
        .get::<tonic::transport::server::TlsConnectInfo<tonic::transport::server::TcpConnectInfo>>()
        && let Some(certs) = tls_info.peer_certs()
        && let Some(cert) = certs.first()
        && let Ok((_, x509)) = X509Certificate::from_der(cert.as_ref())
        && let Some(cn) = x509
            .subject()
            .iter_common_name()
            .next()
            .and_then(|attr| attr.as_str().ok())
    {
        req.extensions_mut().insert(PeerIdentity(cn.to_string()));
    }
    Ok(req)
}

/// Builds a domain [`CompatibilityProfile`] from a named config profile.
///
/// Returns `None` when `id` is empty or `None`. Unknown profile ids and
/// unrecognised capability names are startup errors because they indicate a
/// stale or typo-ridden configuration.
fn build_compatibility_profile(
    profiles: &[Gb28181CompatibilityProfileConfig],
    id: &str,
) -> Result<CompatibilityProfile, String> {
    let profile = profiles
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("unknown compatibility profile id '{id}'"))?;
    let capabilities = profile
        .capabilities
        .iter()
        .map(|c| {
            c.parse::<CompatibilityCapability>()
                .map_err(|_| format!("profile '{}' has unknown capability '{c}'", profile.id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let overrides = build_compatibility_overrides(&profile.overrides).map_err(|e| {
        format!(
            "profile '{}' has invalid compatibility override: {e}",
            profile.id
        )
    })?;
    Ok(CompatibilityProfile {
        profile_id: Some(profile.id.clone()),
        standard_version: profile.standard_version.clone(),
        manufacturer: profile.manufacturer.clone(),
        model: profile.model.clone(),
        firmware: profile.firmware.clone(),
        capabilities,
        evidence_ref: profile.evidence_ref.clone(),
        revision: profile.revision,
        overrides,
    })
}

/// Maps a config-level compatibility override into the typed domain override.
///
/// Empty override sections map to `None` so the strict default behaviour is
/// preserved unless the profile explicitly declares a widening.
fn build_compatibility_overrides(
    config: &Gb28181CompatibilityOverridesConfig,
) -> Result<CompatibilityOverrides, String> {
    let sdp = if config.sdp_allowed_payload_types.is_empty()
        && config.sdp_allowed_attribute_names.is_empty()
    {
        None
    } else {
        Some(SdpMediaOverride {
            allowed_payload_types: config.sdp_allowed_payload_types.clone(),
            allowed_attribute_names: config.sdp_allowed_attribute_names.clone(),
        })
    };
    let broadcast = config
        .broadcast_address_source
        .as_deref()
        .map(|s| {
            s.parse::<BroadcastAddressSource>()
                .map(|address_source| BroadcastOverride { address_source })
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    let media_status = if config.media_status_stopped_codes.is_empty() {
        None
    } else {
        Some(MediaStatusOverride {
            stopped_status_codes: config.media_status_stopped_codes.clone(),
        })
    };
    Ok(CompatibilityOverrides {
        sdp,
        broadcast,
        media_status,
    })
}
