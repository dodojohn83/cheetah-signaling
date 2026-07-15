//! gRPC media control client.

use crate::config::MediaClientConfig;
use crate::error::MediaClientError;
use cheetah_signal_contracts::cheetah::common::v1::command_envelope::Command;
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, EnvelopeMeta, MediaControlExecuteRequest, ResourceKind, ResourceRef, Uuid,
    media_control_client::MediaControlClient as TonicMediaControlClient,
};
use cheetah_signal_contracts::cheetah::media::v1::MediaCommand;
use cheetah_signal_types::{
    MediaBindingId, MediaSessionId, OperationId, OwnerEpoch, TenantId, UtcTimestamp, is_internal_ip,
};
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
    /// Optional wall-clock deadline.
    pub deadline: Option<UtcTimestamp>,
    /// Idempotency key for the command.
    pub idempotency_key: String,
    /// Contract version to advertise.
    pub contract_version: u32,
    /// The media command payload.
    pub command: MediaCommand,
}

enum CircuitState {
    Closed { consecutive_failures: u32 },
    Open(Instant),
}

struct ChannelEntry {
    channel: Channel,
    semaphore: Arc<Semaphore>,
    circuit: Mutex<CircuitState>,
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
    pool: Arc<Mutex<BTreeMap<String, Arc<ChannelEntry>>>>,
}

impl MediaControlClient {
    /// Creates a new media control client.
    pub fn new(config: MediaClientConfig) -> Self {
        Self {
            config,
            pool: Arc::new(Mutex::new(BTreeMap::new())),
        }
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
        let entry = self.get_or_create_entry(endpoint).await?;

        entry.can_attempt()?;

        let permit = self.acquire_permit(&entry, endpoint).await?;

        let deadline = request.deadline;
        let command_envelope = build_command_envelope(request)?;

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
                let needed =
                    time::Duration::milliseconds((self.config.request_timeout_ms + delay) as i64);
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

    async fn get_or_create_entry(
        &self,
        endpoint: &str,
    ) -> Result<Arc<ChannelEntry>, MediaClientError> {
        {
            let pool = self.pool.lock().map_err(|_| {
                MediaClientError::Grpc(Status::internal("connection pool lock poisoned"))
            })?;
            if let Some(entry) = pool.get(endpoint) {
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
            cooldown: Duration::from_millis(self.config.circuit_breaker_cooldown_ms),
            threshold: self.config.circuit_breaker_threshold,
        });

        let mut pool = self.pool.lock().map_err(|_| {
            MediaClientError::Grpc(Status::internal("connection pool lock poisoned"))
        })?;
        if let Some(existing) = pool.get(endpoint) {
            return Ok(Arc::clone(existing));
        }
        if pool.len() >= self.config.max_connections {
            return Err(MediaClientError::PoolExhausted(format!(
                "connection pool limit {} reached",
                self.config.max_connections
            )));
        }
        pool.insert(endpoint.to_string(), Arc::clone(&entry));
        Ok(entry)
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
            if let (Some(cert), Some(key)) = (
                &self.config.tls_client_cert_pem,
                &self.config.tls_client_key_pem,
            ) {
                tls_config =
                    tls_config.identity(Identity::from_pem(cert.as_bytes(), key.as_bytes()));
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
}

impl std::fmt::Debug for MediaControlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaControlClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn build_command_envelope(
    request: MediaControlRequest,
) -> Result<CommandEnvelope, MediaClientError> {
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
    Ok(CommandEnvelope {
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
            occurred_at: Some(to_timestamp(now)?),
            deadline: request.deadline.map(to_timestamp).transpose()?,
            source_node_id: None,
            owner_epoch: request.owner_epoch.0,
            traceparent: String::new(),
            tracestate: String::new(),
            contract_version: request.contract_version,
        }),
        target: Some(target),
        idempotency_key: request.idempotency_key,
        operation_id: request.operation_id.to_string(),
        step_id: request.media_binding_id.to_string(),
        command: Some(Command::MediaCommand(request.command)),
    })
}

fn to_timestamp(ts: UtcTimestamp) -> Result<prost_types::Timestamp, MediaClientError> {
    let offset = ts.as_offset();
    Ok(prost_types::Timestamp {
        seconds: offset.unix_timestamp(),
        nanos: i32::try_from(offset.nanosecond())
            .map_err(|_| MediaClientError::InvalidDeadline("nanos out of range".to_string()))?,
    })
}

fn is_retryable(code: Code) -> bool {
    matches!(
        code,
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted | Code::Aborted
    )
}

fn backoff(base_ms: u64, max_ms: u64, attempt: usize) -> u64 {
    let base = base_ms.saturating_mul(2u64.saturating_pow(attempt as u32));
    if base == 0 {
        return 0;
    }
    fastrand::u64(..=base.min(max_ms))
}
