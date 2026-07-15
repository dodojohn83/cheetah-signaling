//! Shared application state and server entrypoint.

use crate::event_cache::EventCache;
use crate::metrics::RequestMetrics;
use crate::rate_limit::RateLimiter;
use cheetah_domain::ports::{DeviceOwnerResolver, IdGenerator, MediaPort};
use cheetah_message_api::RawEventBus;
use cheetah_signal_application::{
    DeviceService, MediaService, OperationService, WebhookDeliveryConfig, WebhookHttpClient,
    WebhookService,
};
use cheetah_signal_types::config::SecurityConfig;
use cheetah_signal_types::{Clock, NodeId, SecretStore, SignalConfig};
use cheetah_storage_api::Storage;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

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
    /// Allowed CORS origins. Empty disables cross-origin requests.
    pub cors_allowed_origins: Vec<String>,
    /// Rate limit requests per second per (source, tenant, protocol, node).
    pub rate_limit_requests_per_second: u32,
    /// Rate limit burst capacity.
    pub rate_limit_burst: u32,
    /// Webhook delivery worker interval in milliseconds. Zero disables the worker.
    pub webhook_delivery_interval_ms: u64,
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
            read_timeout_ms: u64::try_from(config.http.read_timeout_ms.as_millis()).unwrap_or(5000),
            request_body_limit_bytes: 1024 * 1024,
            cors_allowed_origins: config.http.cors_allowed_origins.clone(),
            rate_limit_requests_per_second: config.http.rate_limit_requests_per_second,
            rate_limit_burst: config.http.rate_limit_burst,
            webhook_delivery_interval_ms: 5000,
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
    /// Webhook application service.
    pub webhook_service: Option<WebhookService>,
    /// Event bus for SSE subscriptions.
    pub event_bus: Arc<dyn RawEventBus>,
    /// Bounded event cache for SSE slow consumers.
    pub event_cache: Arc<EventCache>,
    /// Wall clock and monotonic time source.
    pub clock: Arc<dyn Clock>,
    /// Identifier generator.
    pub id_generator: Arc<dyn IdGenerator>,
    /// Runtime configuration.
    pub config: ApiConfig,
    /// Shared request metrics.
    pub metrics: Arc<RequestMetrics>,
    /// Per-key request rate limiter.
    pub rate_limiter: RateLimiter,
    /// Cancellation token for graceful shutdown.
    pub cancel: CancellationToken,
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
        let rate_limiter = RateLimiter::new(
            config.rate_limit_burst,
            config.rate_limit_requests_per_second,
        );
        let device_service = DeviceService::new(clock.clone(), id_generator.clone());
        let operation_service = OperationService::new(clock.clone(), id_generator.clone());
        let source_node_id = id_generator.generate_node_id();
        let media_service = MediaService::new(
            clock.clone(),
            id_generator.clone(),
            owner_resolver,
            media_port,
            source_node_id,
        );
        Self {
            storage,
            device_service,
            operation_service,
            media_service,
            webhook_service: None,
            event_bus,
            event_cache: EventCache::new(1024),
            clock,
            id_generator,
            config,
            metrics: Arc::new(RequestMetrics::default()),
            rate_limiter,
            cancel: CancellationToken::new(),
        }
    }

    /// Enables the webhook service by wiring a secret store and HTTP client.
    pub fn with_webhook_service(
        mut self,
        secret_store: Arc<dyn SecretStore>,
        http_client: Arc<dyn WebhookHttpClient>,
        config: WebhookDeliveryConfig,
    ) -> Self {
        let service = WebhookService::new(
            self.storage.clone(),
            secret_store,
            self.clock.clone(),
            self.id_generator.clone(),
            http_client,
            config,
        );
        self.webhook_service = Some(service);
        self
    }

    /// Returns the webhook service if configured.
    pub fn webhook_service(&self) -> Result<&WebhookService, crate::HttpError> {
        self.webhook_service
            .as_ref()
            .ok_or_else(|| crate::HttpError::NotImplemented("webhooks not configured".to_string()))
    }
}

/// Running HTTP server handle.
#[derive(Debug)]
pub struct ApiServer {
    /// Bound socket address.
    pub local_addr: SocketAddr,
    /// Server shutdown signal.
    pub shutdown: tokio::sync::oneshot::Sender<()>,
    /// Cancellation token for background tasks.
    pub cancel: CancellationToken,
}

impl ApiServer {
    /// Starts the HTTP server on the configured address.
    pub async fn start(state: ApiState) -> Result<Self, crate::HttpError> {
        let event_bus = state.event_bus.clone();
        let event_cache = state.event_cache.clone();
        let cancel = state.cancel.clone();
        let event_cancel = cancel.child_token();
        tokio::spawn(async move {
            let mut sub = match event_bus
                .subscribe("sig.v1.event.>", "http-api-event-cache")
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("failed to subscribe to event bus: {e}");
                    return;
                }
            };
            loop {
                tokio::select! {
                    _ = event_cancel.cancelled() => break,
                    next = sub.next() => {
                        match next {
                            Ok(Some(delivery)) => {
                                if let Err(e) = event_cache.push(&delivery.envelope) {
                                    tracing::debug!("failed to cache event: {e}");
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                tracing::error!("event subscription error: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });

        if let Some(webhook_service) = state.webhook_service.clone()
            && state.config.webhook_delivery_interval_ms > 0
        {
            let webhook_cancel = cancel.child_token();
            let interval =
                std::time::Duration::from_millis(state.config.webhook_delivery_interval_ms);
            tokio::spawn(async move {
                crate::webhook::run_delivery_worker(webhook_service, webhook_cancel, interval)
                    .await;
            });
        }

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
        let server = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async {
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
            cancel,
        })
    }

    /// Requests a graceful shutdown.
    pub fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.shutdown.send(());
    }
}
