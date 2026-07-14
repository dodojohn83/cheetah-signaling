//! gRPC media control client.

use crate::config::MediaClientConfig;
use crate::error::MediaClientError;
use cheetah_signal_contracts::cheetah::common::v1::command_envelope::Command;
use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, EnvelopeMeta, MediaControlExecuteRequest, ResourceKind, ResourceRef, Uuid,
    media_control_client::MediaControlClient as TonicMediaControlClient,
};
use cheetah_signal_contracts::cheetah::media::v1::MediaCommand;
use cheetah_signal_types::{MediaBindingId, MediaSessionId, OperationId, TenantId, UtcTimestamp};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::{Instant, sleep, timeout};
use tonic::transport::Channel;
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

        let command_envelope = build_command_envelope(request)?;

        let mut last_error: Option<Status> = None;
        for attempt in 0..=self.config.max_retry_attempts {
            if attempt > 0 {
                let delay = backoff(
                    self.config.retry_base_delay_ms,
                    self.config.retry_max_delay_ms,
                    attempt,
                );
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
                    entry.record_failure();
                    return Err(MediaClientError::Grpc(status));
                }
                Err(_) => {
                    let status = Status::deadline_exceeded("media command timed out");
                    if is_retryable(status.code()) && attempt < self.config.max_retry_attempts {
                        last_error = Some(status);
                        continue;
                    }
                    drop(permit);
                    entry.record_failure();
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
        pool.insert(endpoint.to_string(), Arc::clone(&entry));
        Ok(entry)
    }

    async fn connect(&self, endpoint: &str) -> Result<Channel, MediaClientError> {
        let uri = endpoint
            .parse::<tonic::transport::Uri>()
            .map_err(|_| MediaClientError::InvalidEndpoint(endpoint.to_string()))?;
        let endpoint = tonic::transport::Endpoint::new(uri)
            .map_err(MediaClientError::Transport)?
            .connect_timeout(Duration::from_millis(self.config.connect_timeout_ms));
        let channel = endpoint.connect().await?;
        Ok(channel)
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
            owner_epoch: 0,
            traceparent: String::new(),
            tracestate: String::new(),
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
    base.min(max_ms)
}
