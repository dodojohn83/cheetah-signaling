//! Shared application state and server entrypoint.

use crate::metrics::RequestMetrics;
use cheetah_domain::ports::{DeviceOwnerResolver, IdGenerator, MediaPort};
use cheetah_message_api::RawEventBus;
use cheetah_signal_application::{DeviceService, MediaService, OperationService};
use cheetah_signal_types::config::SecurityConfig;
use cheetah_signal_types::{Clock, NodeId, SignalConfig};
use cheetah_storage_api::Storage;
use std::net::SocketAddr;
use std::sync::Arc;

/// Configuration subset used by the HTTP API.
#[derive(Clone, Debug)]
pub struct ApiConfig {
    /// HTTP bind address.
    pub listen_addr: String,
    /// HTTP port.
    pub port: u16,
    /// Request read timeout.
    pub read_timeout_ms: u64,
    /// Maximum request body size in bytes.
    pub request_body_limit_bytes: usize,
    /// Process node identifier.
    pub node_id: NodeId,
    /// Security settings.
    pub security: SecurityConfig,
}

impl From<&SignalConfig> for ApiConfig {
    fn from(config: &SignalConfig) -> Self {
        Self {
            listen_addr: config.http.listen_addr.clone(),
            port: config.http.port,
            read_timeout_ms: config.http.read_timeout_ms.as_millis() as u64,
            request_body_limit_bytes: 1024 * 1024,
            node_id: config.system.node_id.unwrap_or_default(),
            security: config.security.clone(),
        }
    }
}

/// State shared by all HTTP handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Storage port.
    pub storage: Arc<dyn Storage>,
    /// Device application service.
    pub device_service: DeviceService,
    /// Operation application service.
    pub operation_service: OperationService,
    /// Media application service.
    pub media_service: MediaService,
    /// Event bus for SSE subscriptions.
    pub event_bus: Arc<dyn RawEventBus>,
    /// Wall clock and monotonic time source.
    pub clock: Arc<dyn Clock>,
    /// Identifier generator.
    pub id_generator: Arc<dyn IdGenerator>,
    /// Runtime configuration.
    pub config: ApiConfig,
    /// Shared request metrics.
    pub metrics: Arc<RequestMetrics>,
}

impl std::fmt::Debug for ApiState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ApiState {
    /// Creates a new API state.
    pub fn new(
        config: ApiConfig,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
        id_generator: Arc<dyn IdGenerator>,
        event_bus: Arc<dyn RawEventBus>,
        owner_resolver: Arc<dyn DeviceOwnerResolver>,
        media_port: Arc<dyn MediaPort>,
    ) -> Self {
        let device_service = DeviceService::new(clock.clone(), id_generator.clone());
        let operation_service = OperationService::new(clock.clone(), id_generator.clone());
        let media_service = MediaService::new(
            clock.clone(),
            id_generator.clone(),
            owner_resolver,
            media_port,
        );
        Self {
            storage,
            device_service,
            operation_service,
            media_service,
            event_bus,
            clock,
            id_generator,
            config,
            metrics: Arc::new(RequestMetrics::default()),
        }
    }
}

/// Running HTTP server handle.
#[derive(Debug)]
pub struct ApiServer {
    /// Bound socket address.
    pub local_addr: SocketAddr,
    /// Server shutdown signal.
    pub shutdown: tokio::sync::oneshot::Sender<()>,
}

impl ApiServer {
    /// Starts the HTTP server on the configured address.
    pub async fn start(state: ApiState) -> Result<Self, crate::HttpError> {
        let router = crate::router::build_router(state.clone());
        let addr: SocketAddr = format!("{}:{}", state.config.listen_addr, state.config.port)
            .parse()
            .map_err(|e| crate::HttpError::Internal(format!("invalid listen address: {e}")))?;
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            crate::HttpError::Internal(format!("failed to bind HTTP listener: {e}"))
        })?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| crate::HttpError::Internal(format!("failed to get local address: {e}")))?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let server = axum::serve(listener, router).with_graceful_shutdown(async {
            let _ = rx.await;
        });
        tokio::spawn(async move {
            if let Err(e) = server.await {
                tracing::error!("HTTP server error: {e}");
            }
        });
        Ok(Self {
            local_addr,
            shutdown: tx,
        })
    }

    /// Requests a graceful shutdown.
    pub fn shutdown(self) {
        let _ = self.shutdown.send(());
    }
}
