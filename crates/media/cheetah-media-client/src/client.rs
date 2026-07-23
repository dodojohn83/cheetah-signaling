//! gRPC media control client.

use crate::config::MediaClientConfig;
use crate::error::MediaClientError;
use cheetah_signal_contracts::cheetah::common::v1::command_envelope::Command;
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, EnvelopeMeta, ListSessionsRequest, MediaControlExecuteRequest, ResourceKind,
    ResourceRef, Uuid, media_control_client::MediaControlClient as TonicMediaControlClient,
    media_query_client::MediaQueryClient as TonicMediaQueryClient,
};
use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCommand, MediaEvent, SubscribeRequest,
    media_event_stream_service_client::MediaEventStreamServiceClient,
};
use cheetah_signal_types::{
    MediaBindingId, MediaNodeInstanceEpoch, MediaSessionId, NodeId, OperationId, OwnerEpoch,
    SecretStore, TenantId, UtcTimestamp, is_internal_ip,
};
use secrecy::ExposeSecret;
use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, sleep, timeout};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity, Uri};
use tonic::{Code, Request, Status};

/// A request to execute a media command on a media node.
#[derive(Clone, Debug)]
pub struct MediaControlRequest {
    /// Request / correlation identifier.
    pub request_id: String,
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Media session identifier.
    pub media_session_id: MediaSessionId,
    /// Media binding identifier.
    pub media_binding_id: MediaBindingId,
    /// Operation identifier.
    pub operation_id: OperationId,
    /// Owner epoch of the device/session for fencing.
    pub owner_epoch: OwnerEpoch,
    /// Signaling node that owns this command.
    pub source_node_id: NodeId,
    /// Target media node identifier.
    pub media_node_id: NodeId,
    /// Target media node instance epoch for fencing.
    pub target_media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Optional wall-clock deadline.
    pub deadline: Option<UtcTimestamp>,
    /// Idempotency key for the command.
    pub idempotency_key: String,
    /// Contract version to advertise.
    pub contract_version: u32,
    /// The media command payload.
    pub command: MediaCommand,
}

/// A request to list active sessions on a media node.
#[derive(Clone, Debug)]
pub struct MediaListSessionsRequest {
    /// Media node identifier.
    pub media_node_id: NodeId,
    /// Media node instance epoch for fencing and connection keying.
    pub media_node_instance_epoch: MediaNodeInstanceEpoch,
    /// Tenant identifier.
    pub tenant_id: TenantId,
    /// Maximum number of sessions to return.
    pub page_size: u32,
    /// Opaque page cursor.
    pub page_token: Option<String>,
}

enum CircuitState {
    Closed { consecutive_failures: u32 },
    Open(Instant),
}

struct ChannelEntry {
    channel: Channel,
    semaphore: Arc<Semaphore>,
    circuit: Mutex<CircuitState>,
    last_used: Mutex<Instant>,
    cooldown: Duration,
    threshold: u32,
}

impl ChannelEntry {
    fn can_attempt(&self) -> Result<(), MediaClientError> {
        let mut state = self.circuit.lock().map_err(|_| {
            MediaClientError::Grpc(Status::internal("circuit breaker lock poisoned"))
        })?;
        match *state {
            CircuitState::Closed { .. } => Ok(()),
            CircuitState::Open(until) => {
                if Instant::now() >= until {
                    *state = CircuitState::Closed {
                        consecutive_failures: 0,
                    };
                    Ok(())
                } else {
                    Err(MediaClientError::CircuitOpen("endpoint".to_string()))
                }
            }
        }
    }

    fn record_success(&self) {
        if let Ok(mut state) = self.circuit.lock() {
            *state = CircuitState::Closed {
                consecutive_failures: 0,
            };
        }
    }

    fn touch(&self) {
        if let Ok(mut t) = self.last_used.lock() {
            *t = Instant::now();
        }
    }

    fn last_used(&self) -> Instant {
        *self.last_used.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn record_failure(&self) {
        if let Ok(mut state) = self.circuit.lock() {
            match *state {
                CircuitState::Closed {
                    consecutive_failures,
                } => {
                    let next = consecutive_failures + 1;
                    if next >= self.threshold {
                        *state = CircuitState::Open(Instant::now() + self.cooldown);
                    } else {
                        *state = CircuitState::Closed {
                            consecutive_failures: next,
                        };
                    }
                }
                CircuitState::Open(_until) => {
                    *state = CircuitState::Open(Instant::now() + self.cooldown);
                }
            }
        }
    }
}

/// Media control gRPC client with connection pooling, retries, and circuit breaker.
#[derive(Clone)]
pub struct MediaControlClient {
    config: MediaClientConfig,
    secret_store: Option<Arc<dyn SecretStore>>,
    pool: Arc<Mutex<BTreeMap<String, Arc<ChannelEntry>>>>,
}

impl MediaControlClient {
    /// Creates a new media control client.
    pub fn new(config: MediaClientConfig) -> Self {
        Self {
            config,
            secret_store: None,
            pool: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Attaches a secret store that resolves the TLS client key by reference.
    pub fn with_secret_store(mut self, secret_store: Arc<dyn SecretStore>) -> Self {
        self.secret_store = Some(secret_store);
        self
    }

    /// Executes a media command against the given media node endpoint.
    pub async fn execute(
        &self,
        endpoint: &str,
        request: MediaControlRequest,
    ) -> Result<
        cheetah_signal_contracts::cheetah::common::v1::MediaControlExecuteResponse,
        MediaClientError,
    > {
        validate_media_target(
            endpoint,
            request.media_node_id,
            request.target_media_node_instance_epoch,
        )?;
        let key = self.pool_key(
            endpoint,
            request.media_node_id,
            request.target_media_node_instance_epoch,
        );
        let entry = self
            .get_or_create_entry(
                &key,
                endpoint,
                request.media_node_id,
                request.target_media_node_instance_epoch,
            )
            .await?;

        entry.can_attempt()?;

        let permit = self.acquire_permit(&entry, endpoint).await?;

        let deadline = request.deadline;
        let command_envelope = build_command_envelope(request);

        let mut last_error: Option<Status> = None;
        for attempt in 0..=self.config.max_retry_attempts {
            let delay = if attempt == 0 {
                0
            } else {
                backoff(
                    self.config.retry_base_delay_ms,
                    self.config.retry_max_delay_ms,
                    attempt,
                )
            };

            if let Some(deadline) = deadline {
                let now = UtcTimestamp::from_offset(time::OffsetDateTime::now_utc());
                let total_ms = self.config.request_timeout_ms.saturating_add(delay);
                let total_ms_i64 = i64::try_from(total_ms).unwrap_or(i64::MAX);
                let needed = time::Duration::milliseconds(total_ms_i64);
                if now.as_offset() + needed >= deadline.as_offset() {
                    return Err(MediaClientError::Grpc(last_error.unwrap_or_else(|| {
                        Status::deadline_exceeded("media command deadline exceeded")
                    })));
                }
            }

            if delay > 0 {
                sleep(Duration::from_millis(delay)).await;
            }

            let mut client = TonicMediaControlClient::new(entry.channel.clone());
            let body = MediaControlExecuteRequest {
                command: Some(command_envelope.clone()),
            };
            let grpc_request = Request::new(body);
            let result = timeout(
                Duration::from_millis(self.config.request_timeout_ms),
                client.execute(grpc_request),
            )
            .await;

            match result {
                Ok(Ok(response)) => {
                    drop(permit);
                    entry.record_success();
                    return Ok(response.into_inner());
                }
                Ok(Err(status)) => {
                    if is_retryable(status.code()) && attempt < self.config.max_retry_attempts {
                        last_error = Some(status);
                        continue;
                    }
                    drop(permit);
                    if is_retryable(status.code()) {
                        entry.record_failure();
                    }
                    return Err(MediaClientError::Grpc(status));
                }
                Err(_) => {
                    let status = Status::deadline_exceeded("media command timed out");
                    if is_retryable(status.code()) && attempt < self.config.max_retry_attempts {
                        last_error = Some(status);
                        continue;
                    }
                    drop(permit);
                    if is_retryable(status.code()) {
                        entry.record_failure();
                    }
                    return Err(MediaClientError::Grpc(status));
                }
            }
        }

        drop(permit);
        entry.record_failure();
        Err(MediaClientError::Grpc(last_error.unwrap_or_else(|| {
            Status::unavailable("media command failed after retries")
        })))
    }

    /// Lists active sessions on the media node at the given endpoint.
    pub async fn list_sessions(
        &self,
        endpoint: &str,
        request: MediaListSessionsRequest,
    ) -> Result<cheetah_signal_contracts::cheetah::common::v1::ListSessionsResponse, MediaClientError>
    {
        validate_media_target(
            endpoint,
            request.media_node_id,
            request.media_node_instance_epoch,
        )?;
        let key = self.pool_key(
            endpoint,
            request.media_node_id,
            request.media_node_instance_epoch,
        );
        let entry = self
            .get_or_create_entry(
                &key,
                endpoint,
                request.media_node_id,
                request.media_node_instance_epoch,
            )
            .await?;

        entry.can_attempt()?;

        let permit = self.acquire_permit(&entry, endpoint).await?;

        let body = ListSessionsRequest {
            media_node_id: request.media_node_id.to_string(),
            tenant_id: request.tenant_id.to_string(),
            page_size: request.page_size,
            page_token: request.page_token.unwrap_or_default(),
        };

        let mut last_error: Option<Status> = None;
        for attempt in 0..=self.config.max_retry_attempts {
            let delay = if attempt == 0 {
                0
            } else {
                backoff(
                    self.config.retry_base_delay_ms,
                    self.config.retry_max_delay_ms,
                    attempt,
                )
            };

            if delay > 0 {
                sleep(Duration::from_millis(delay)).await;
            }

            let mut client = TonicMediaQueryClient::new(entry.channel.clone());
            let grpc_request = Request::new(body.clone());
            let result = timeout(
                Duration::from_millis(self.config.request_timeout_ms),
                client.list_sessions(grpc_request),
            )
            .await;

            match result {
                Ok(Ok(response)) => {
                    drop(permit);
                    entry.record_success();
                    return Ok(response.into_inner());
                }
                Ok(Err(status)) => {
                    if is_retryable(status.code()) && attempt < self.config.max_retry_attempts {
                        last_error = Some(status);
                        continue;
                    }
                    drop(permit);
                    if is_retryable(status.code()) {
                        entry.record_failure();
                    }
                    return Err(MediaClientError::Grpc(status));
                }
                Err(_) => {
                    let status = Status::deadline_exceeded("media session list timed out");
                    if is_retryable(status.code()) && attempt < self.config.max_retry_attempts {
                        last_error = Some(status);
                        continue;
                    }
                    drop(permit);
                    if is_retryable(status.code()) {
                        entry.record_failure();
                    }
                    return Err(MediaClientError::Grpc(status));
                }
            }
        }

        drop(permit);
        entry.record_failure();
        Err(MediaClientError::Grpc(last_error.unwrap_or_else(|| {
            Status::unavailable("media session list failed after retries")
        })))
    }

    async fn get_or_create_entry(
        &self,
        key: &str,
        endpoint: &str,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
    ) -> Result<Arc<ChannelEntry>, MediaClientError> {
        {
            let pool = self.pool.lock().map_err(|_| {
                MediaClientError::Grpc(Status::internal("connection pool lock poisoned"))
            })?;
            if let Some(entry) = pool.get(key) {
                entry.touch();
                return Ok(Arc::clone(entry));
            }
        }

        let channel = self.connect(endpoint).await?;
        let entry = Arc::new(ChannelEntry {
            channel,
            semaphore: Arc::new(Semaphore::new(self.config.per_node_concurrency)),
            circuit: Mutex::new(CircuitState::Closed {
                consecutive_failures: 0,
            }),
            last_used: Mutex::new(Instant::now()),
            cooldown: Duration::from_millis(self.config.circuit_breaker_cooldown_ms),
            threshold: self.config.circuit_breaker_threshold,
        });

        let mut pool = self.pool.lock().map_err(|_| {
            MediaClientError::Grpc(Status::internal("connection pool lock poisoned"))
        })?;
        if let Some(existing) = pool.get(key) {
            existing.touch();
            return Ok(Arc::clone(existing));
        }

        // If the same media node is now reached through a different endpoint,
        // TLS identity, or newer instance epoch, close the stale channel(s) so
        // the pool does not keep an obsolete connection open after rotation.
        let node_id_str = media_node_id.to_string();
        let new_epoch = media_node_instance_epoch.0;
        let stale: Vec<String> = pool
            .keys()
            .filter(|k| {
                if *k == key {
                    return false;
                }
                let mut parts = k.split('\0');
                let same_node = parts.nth(1) == Some(&node_id_str);
                let stale_or_older_epoch = parts
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|epoch| epoch <= new_epoch);
                same_node && stale_or_older_epoch
            })
            .cloned()
            .collect();
        for stale_key in stale {
            pool.remove(&stale_key);
        }

        if pool.len() >= self.config.max_connections
            && let Some((oldest_key, _)) = pool.iter().min_by_key(|(_, e)| e.last_used())
        {
            let oldest_key = oldest_key.clone();
            pool.remove(&oldest_key);
        }
        pool.insert(key.to_string(), Arc::clone(&entry));
        Ok(entry)
    }

    fn pool_key(
        &self,
        endpoint: &str,
        media_node_id: NodeId,
        media_node_instance_epoch: MediaNodeInstanceEpoch,
    ) -> String {
        format!(
            "{}\0{}\0{}\0{}",
            endpoint,
            media_node_id,
            media_node_instance_epoch.0,
            self.tls_identity_digest()
        )
    }

    fn tls_identity_digest(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.config.tls_ca_pem.hash(&mut hasher);
        self.config.tls_client_cert_pem.hash(&mut hasher);
        self.config.tls_client_key_secret_name.hash(&mut hasher);
        self.config.allow_insecure_http.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    async fn connect(&self, endpoint: &str) -> Result<Channel, MediaClientError> {
        let uri = endpoint
            .parse::<Uri>()
            .map_err(|_| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        let scheme = uri
            .scheme_str()
            .ok_or_else(|| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        let host = uri
            .host()
            .ok_or_else(|| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        if host.is_empty() {
            return Err(MediaClientError::InvalidEndpoint(endpoint.to_string()));
        }

        let is_http = scheme.eq_ignore_ascii_case("http");
        let is_https = scheme.eq_ignore_ascii_case("https");
        if !is_http && !is_https {
            return Err(MediaClientError::InvalidEndpoint(format!(
                "unsupported scheme '{scheme}'"
            )));
        }
        if is_http && !self.config.allow_insecure_http {
            return Err(MediaClientError::InsecureEndpoint(endpoint.to_string()));
        }

        let default_port = if is_https { 443 } else { 80 };
        let port = uri.port_u16().unwrap_or(default_port);

        let target_addr = self.resolve_and_validate(host, port).await?;

        let literal_uri = format!("{scheme}://{target_addr}")
            .parse::<Uri>()
            .map_err(|_| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        let mut builder = Endpoint::new(literal_uri)
            .map_err(MediaClientError::Transport)?
            .connect_timeout(Duration::from_millis(self.config.connect_timeout_ms));

        let authority = if host.contains(':') {
            format!("{scheme}://[{host}]:{port}")
        } else {
            format!("{scheme}://{host}:{port}")
        };
        let authority_uri = authority
            .parse::<Uri>()
            .map_err(|_| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        builder = builder.origin(authority_uri);

        if is_https {
            let mut tls_config = ClientTlsConfig::new().with_enabled_roots();
            if let Some(ca) = &self.config.tls_ca_pem {
                tls_config = tls_config.ca_certificate(Certificate::from_pem(ca.as_bytes()));
            }
            if let Some(cert_pem) = &self.config.tls_client_cert_pem {
                let key_pem = match &self.config.tls_client_key_secret_name {
                    Some(key_name) => {
                        let store = self.secret_store.as_ref().ok_or_else(|| {
                            MediaClientError::TlsConfig(
                                "secret store not configured for mTLS client key".to_string(),
                            )
                        })?;
                        store
                            .get(key_name)
                            .map_err(|e| {
                                MediaClientError::TlsConfig(format!(
                                    "failed to load client key secret: {e}"
                                ))
                            })?
                            .expose_secret()
                            .to_string()
                    }
                    None => {
                        return Err(MediaClientError::TlsConfig(
                            "mTLS client certificate provided without secret key name".to_string(),
                        ));
                    }
                };
                tls_config = tls_config
                    .identity(Identity::from_pem(cert_pem.as_bytes(), key_pem.as_bytes()));
            }
            tls_config = tls_config.domain_name(host.to_string());
            builder = builder
                .tls_config(tls_config)
                .map_err(MediaClientError::Transport)?;
        }

        Ok(builder.connect().await?)
    }

    async fn resolve_and_validate(
        &self,
        host: &str,
        port: u16,
    ) -> Result<SocketAddr, MediaClientError> {
        if let Ok(ip) = IpAddr::from_str(host) {
            if !self.config.allow_internal_endpoints && is_internal_ip(ip) {
                return Err(MediaClientError::InternalEndpoint(format!("{host}:{port}")));
            }
            return Ok(SocketAddr::new(ip, port));
        }

        let lookup = timeout(
            Duration::from_millis(self.config.endpoint_dns_lookup_timeout_ms),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| MediaClientError::InvalidEndpoint(format!("DNS lookup timed out for {host}")))?
        .map_err(|e| {
            MediaClientError::InvalidEndpoint(format!("DNS lookup failed for {host}: {e}"))
        })?;

        let mut chosen = None;
        for addr in lookup {
            if self.config.allow_internal_endpoints || !is_internal_ip(addr.ip()) {
                chosen = Some(addr);
                break;
            }
        }

        chosen.ok_or_else(|| {
            MediaClientError::InternalEndpoint(format!(
                "{host}:{port} resolved only to internal addresses"
            ))
        })
    }

    async fn acquire_permit(
        &self,
        entry: &ChannelEntry,
        endpoint: &str,
    ) -> Result<OwnedSemaphorePermit, MediaClientError> {
        let permit = timeout(
            Duration::from_millis(self.config.request_timeout_ms),
            entry.semaphore.clone().acquire_owned(),
        )
        .await
        .map_err(|_| {
            MediaClientError::Grpc(Status::resource_exhausted(format!(
                "media node {endpoint} concurrency limit timeout"
            )))
        })?
        .map_err(|_| MediaClientError::Grpc(Status::internal("semaphore closed")))?;
        Ok(permit)
    }

    /// Subscribes to a media node's event stream.
    pub async fn subscribe(
        &self,
        endpoint: &str,
        request: SubscribeRequest,
    ) -> Result<tonic::codec::Streaming<MediaEvent>, MediaClientError> {
        let channel = self.connect(endpoint).await?;
        let mut client = MediaEventStreamServiceClient::new(channel);
        let response = client
            .subscribe(request)
            .await
            .map_err(MediaClientError::Grpc)?;
        Ok(response.into_inner())
    }
}

impl std::fmt::Debug for MediaControlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaControlClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Validates endpoint and fencing identifiers before establishing a connection.
fn validate_media_target(
    endpoint: &str,
    media_node_id: NodeId,
    instance_epoch: MediaNodeInstanceEpoch,
) -> Result<(), MediaClientError> {
    if endpoint.trim().is_empty() {
        return Err(MediaClientError::InvalidEndpoint(
            "endpoint must not be empty".to_string(),
        ));
    }
    if media_node_id.as_uuid().is_nil() {
        return Err(MediaClientError::MissingIdentifier {
            field: "media_node_id",
            reason: "must not be nil".to_string(),
        });
    }
    if instance_epoch.0 == 0 {
        return Err(MediaClientError::MissingIdentifier {
            field: "media_node_instance_epoch",
            reason: "must be non-zero for fencing".to_string(),
        });
    }
    let uri = endpoint
        .parse::<Uri>()
        .map_err(|_| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
    let scheme = uri
        .scheme_str()
        .ok_or_else(|| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err(MediaClientError::InvalidEndpoint(format!(
            "unsupported scheme '{scheme}'"
        )));
    }
    Ok(())
}

fn build_command_envelope(request: MediaControlRequest) -> CommandEnvelope {
    let now = UtcTimestamp::from_offset(time::OffsetDateTime::now_utc());
    let target = ResourceRef {
        tenant_id: Some(Uuid {
            value: request.tenant_id.to_string(),
        }),
        kind: ResourceKind::MediaSession as i32,
        resource_id: Some(Uuid {
            value: request.media_session_id.to_string(),
        }),
    };
    let mut command = request.command.clone();
    command.target_media_node_instance_epoch = request.target_media_node_instance_epoch.0;
    CommandEnvelope {
        meta: Some(EnvelopeMeta {
            message_id: Some(Uuid {
                value: request.request_id.clone(),
            }),
            tenant_id: Some(Uuid {
                value: request.tenant_id.to_string(),
            }),
            correlation_id: Some(Uuid {
                value: request.request_id,
            }),
            causation_id: Some(Uuid {
                value: request.operation_id.to_string(),
            }),
            occurred_at: Some(now.to_prost_timestamp()),
            deadline: request.deadline.map(|ts| ts.to_prost_timestamp()),
            source_node_id: Some(Uuid {
                value: request.source_node_id.to_string(),
            }),
            owner_epoch: request.owner_epoch.0,
            traceparent: String::new(),
            tracestate: String::new(),
            contract_version: request.contract_version,
        }),
        target: Some(target),
        idempotency_key: request.idempotency_key,
        operation_id: request.operation_id.to_string(),
        step_id: request.media_binding_id.to_string(),
        command: Some(Command::MediaCommand(command)),
    }
}

fn is_retryable(code: Code) -> bool {
    matches!(
        code,
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted | Code::Aborted
    )
}

fn backoff(base_ms: u64, max_ms: u64, attempt: usize) -> u64 {
    let exponent = u32::try_from(attempt).unwrap_or(u32::MAX);
    let base = base_ms.saturating_mul(2u64.saturating_pow(exponent));
    if base == 0 {
        return 0;
    }
    fastrand::u64(..=base.min(max_ms))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn backoff_caps_attempt_exponent_at_u32_max() {
        // Values beyond u32::MAX must not wrap the exponent back to a tiny delay.
        let huge = (u32::MAX as usize).saturating_add(1);
        let value = backoff(1, u64::MAX, huge);
        assert!(value > 0, "huge retry attempts must still produce a non-zero backoff");
    }
}
