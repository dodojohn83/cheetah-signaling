//! Domain `MediaPort` implementation backed by the media scheduler.

use crate::scheduler::MediaScheduler;
use cheetah_domain::{
    DomainError, MediaNodeCommand, MediaNodeCommandResult, MediaNodeSessionRef, MediaPort,
    MediaRequirements, MediaReservation,
};
use cheetah_media_client::{
    MediaClientError, MediaControlClient, MediaControlRequest, MediaListSessionsRequest,
};
use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCommand, MediaControlPayload, MediaMutationContext, media_command,
};
use cheetah_signal_types::MediaNodeInstanceEpoch;
use cheetah_signal_types::Page;
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, MediaBindingId, MediaSessionId, MessageId, NodeId, PageRequest,
    TenantId, UtcTimestamp,
};
use std::str::FromStr;
use std::sync::Arc;

use crate::error::SchedulerError;

/// A `MediaPort` that selects media nodes using the scheduler and dispatches
/// commands to them via the media control client.
#[derive(Clone)]
pub struct SchedulerMediaPort {
    scheduler: Arc<dyn MediaScheduler>,
    client: MediaControlClient,
}

impl SchedulerMediaPort {
    /// Creates a new scheduler-backed media port.
    pub fn new(scheduler: Arc<dyn MediaScheduler>, client: MediaControlClient) -> Self {
        Self { scheduler, client }
    }
}

impl std::fmt::Debug for SchedulerMediaPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerMediaPort").finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl MediaPort for SchedulerMediaPort {
    async fn reserve_live(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        _purpose: cheetah_domain::MediaPurpose,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaReservation, DomainError> {
        reserve(
            self.scheduler.clone(),
            tenant_id,
            media_binding_id,
            requirements,
            clock,
        )
        .await
    }

    async fn reserve_playback(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        _start_time: UtcTimestamp,
        _end_time: UtcTimestamp,
        _scale: f64,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaReservation, DomainError> {
        reserve(
            self.scheduler.clone(),
            tenant_id,
            media_binding_id,
            requirements,
            clock,
        )
        .await
    }

    async fn reserve_talk(
        &self,
        tenant_id: TenantId,
        _device_id: DeviceId,
        _channel_id: ChannelId,
        _media_session_id: MediaSessionId,
        media_binding_id: MediaBindingId,
        requirements: &MediaRequirements,
        clock: &dyn Clock,
    ) -> Result<MediaReservation, DomainError> {
        reserve(
            self.scheduler.clone(),
            tenant_id,
            media_binding_id,
            requirements,
            clock,
        )
        .await
    }

    async fn release(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<(), DomainError> {
        self.scheduler
            .release(tenant_id, media_binding_id, clock)
            .await
            .map_err(map_scheduler_error)
    }

    async fn execute(
        &self,
        command: MediaNodeCommand,
        clock: &dyn Clock,
    ) -> Result<MediaNodeCommandResult, DomainError> {
        let node = self
            .scheduler
            .get_node(command.media_node_id, clock)
            .await
            .ok_or_else(|| {
                DomainError::not_found("media_node", command.media_node_id.to_string())
            })?;

        if node.instance_epoch != command.media_node_instance_epoch.0 {
            return Err(DomainError::unavailable(
                "media node instance epoch mismatch",
            ));
        }

        let endpoint = node.control_endpoint;
        let contract_version = if command.contract_version > 0 {
            command.contract_version
        } else {
            node.contract_version
        };

        let payload = serde_json::to_vec(&command.payload).map_err(|e| {
            DomainError::internal(format!("failed to serialize media command payload: {e}"))
        })?;
        let deadline = command.deadline.map(|d| d.as_timestamp().to_prost_timestamp());
        let context = MediaMutationContext {
            tenant_id: command.tenant_id.to_string(),
            request_id: command.request_id.clone(),
            correlation_id: command.request_id.clone(),
            message_id: MessageId::generate().to_string(),
            idempotency_key: command.idempotency_key.clone(),
            deadline,
            source_signaling_node_id: command.source_node_id.to_string(),
            owner_epoch: command.owner_epoch.0,
            target_media_node_id: command.media_node_id.to_string(),
            target_media_node_instance_epoch: command.media_node_instance_epoch.0,
            operation_id: command.operation_id.to_string(),
            operation_step_id: command.payload.kind().to_string(),
            media_session_id: Some(command.media_session_id.to_string()),
            media_binding_id: Some(command.media_binding_id.to_string()),
            contract_version: contract_version as u64,
            traceparent: None,
            tracestate: None,
        };
        let proto_command = MediaCommand {
            command: Some(media_command::Command::Control(MediaControlPayload {
                media_session_id: command.media_session_id.to_string(),
                command_type: command.payload.kind().to_string(),
                payload,
            })),
            target_media_node_instance_epoch: command.media_node_instance_epoch.0,
            context: Some(context),
        };

        let request = MediaControlRequest {
            request_id: command.request_id,
            tenant_id: command.tenant_id,
            media_session_id: command.media_session_id,
            media_binding_id: command.media_binding_id,
            operation_id: command.operation_id,
            owner_epoch: command.owner_epoch,
            source_node_id: command.source_node_id,
            target_media_node_instance_epoch: command.media_node_instance_epoch,
            deadline: command.deadline.map(|d| d.as_timestamp()),
            idempotency_key: command.idempotency_key,
            contract_version,
            command: proto_command,
        };

        let response = self
            .client
            .execute(&endpoint, request)
            .await
            .map_err(map_client_error)?;

        let result = response
            .result
            .ok_or_else(|| DomainError::unavailable("media node returned no command result"))?;

        match cheetah_signal_contracts::cheetah::common::v1::CommandStatus::try_from(result.status)
        {
            Ok(cheetah_signal_contracts::cheetah::common::v1::CommandStatus::Completed) => {
                Ok(MediaNodeCommandResult::Completed)
            }
            Ok(cheetah_signal_contracts::cheetah::common::v1::CommandStatus::Accepted) => {
                Ok(MediaNodeCommandResult::Accepted)
            }
            Ok(s) => Ok(MediaNodeCommandResult::Failed {
                code: format!("{s:?}"),
                message: result
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_default(),
            }),
            Err(_) => Ok(MediaNodeCommandResult::Failed {
                code: "unknown_status".to_string(),
                message: result
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_default(),
            }),
        }
    }

    async fn list_nodes(
        &self,
        _tenant_id: TenantId,
        clock: &dyn Clock,
    ) -> Result<Vec<NodeId>, DomainError> {
        Ok(self.scheduler.list_nodes(clock).await)
    }

    async fn list_sessions(
        &self,
        tenant_id: TenantId,
        media_node_id: NodeId,
        page: PageRequest,
        clock: &dyn Clock,
    ) -> Result<Page<MediaNodeSessionRef>, DomainError> {
        let node = self
            .scheduler
            .get_node(media_node_id, clock)
            .await
            .ok_or_else(|| DomainError::not_found("media_node", media_node_id.to_string()))?;

        let endpoint = node.control_endpoint;
        let request = MediaListSessionsRequest {
            media_node_id,
            tenant_id,
            page_size: page.page_size,
            page_token: page.cursor,
        };

        let response = self
            .client
            .list_sessions(&endpoint, request)
            .await
            .map_err(map_client_error)?;

        let mut items = Vec::with_capacity(response.sessions.len());
        for proto in response.sessions {
            let media_session_id = match MediaSessionId::from_str(&proto.media_session_id) {
                Ok(id) => id,
                Err(_) => continue,
            };
            let device_id = if proto.device_id.is_empty() {
                None
            } else {
                DeviceId::from_str(&proto.device_id).ok()
            };
            let channel_id = if proto.channel_id.is_empty() {
                None
            } else {
                ChannelId::from_str(&proto.channel_id).ok()
            };
            items.push(MediaNodeSessionRef {
                media_session_id,
                device_id,
                channel_id,
                media_node_instance_epoch: MediaNodeInstanceEpoch(proto.media_node_instance_epoch),
            });
        }

        let next_cursor = if response.next_page_token.is_empty() {
            None
        } else {
            Some(response.next_page_token)
        };

        Ok(Page {
            items,
            next_cursor,
            total: None,
        })
    }
}

async fn reserve(
    scheduler: Arc<dyn MediaScheduler>,
    tenant_id: TenantId,
    binding_id: MediaBindingId,
    requirements: &MediaRequirements,
    clock: &dyn Clock,
) -> Result<MediaReservation, DomainError> {
    let mut excluded = Vec::new();
    let max_attempts = scheduler.config().max_reserve_attempts.max(1);

    for _ in 0..max_attempts {
        let node = scheduler
            .schedule(tenant_id, requirements, &excluded, clock)
            .await
            .map_err(|e| {
                if matches!(e, crate::error::SchedulerError::NoNode(_))
                    && requirements.operation == "talk"
                {
                    DomainError::not_supported("talk not supported by any media node")
                } else {
                    map_scheduler_error(e)
                }
            })?;
        let node_id = node.node_id;
        let instance_epoch = node.instance_epoch_value();

        match scheduler
            .reserve(node_id, tenant_id, binding_id, requirements, clock)
            .await
        {
            Ok(_) => {
                return Ok(MediaReservation {
                    media_node_id: node_id,
                    media_node_instance_epoch: instance_epoch,
                    contract_version: node.contract_version,
                });
            }
            Err(SchedulerError::CapacityExhausted(_)) => {
                excluded.push(node_id);
                continue;
            }
            Err(e) => return Err(map_scheduler_error(e)),
        }
    }

    Err(DomainError::unavailable(
        "no media node had capacity after retries",
    ))
}

fn map_scheduler_error(e: crate::error::SchedulerError) -> DomainError {
    match e {
        crate::error::SchedulerError::NoNode(_)
        | crate::error::SchedulerError::CapacityExhausted(_) => {
            DomainError::unavailable(e.to_string())
        }
        crate::error::SchedulerError::NodeNotFound(_)
        | crate::error::SchedulerError::ReservationNotFound { .. } => {
            DomainError::not_found("media_node", e.to_string())
        }
        crate::error::SchedulerError::InvalidArgument(_)
        | crate::error::SchedulerError::IdentityMismatch { .. } => {
            DomainError::invalid_argument(e.to_string())
        }
    }
}

fn map_client_error(e: MediaClientError) -> DomainError {
    match e {
        MediaClientError::InvalidEndpoint(_)
        | MediaClientError::InsecureEndpoint(_)
        | MediaClientError::InternalEndpoint(_)
        | MediaClientError::MissingIdentifier { .. }
        | MediaClientError::InvalidDeadline(_) => DomainError::invalid_argument(e.to_string()),
        MediaClientError::Grpc(ref status) => match status.code() {
            tonic::Code::InvalidArgument => {
                DomainError::invalid_argument(status.message().to_string())
            }
            tonic::Code::NotFound => DomainError::not_found("media", status.message().to_string()),
            tonic::Code::AlreadyExists => {
                DomainError::invalid_argument(status.message().to_string())
            }
            _ => DomainError::unavailable(e.to_string()),
        },
        MediaClientError::Transport(_)
        | MediaClientError::CircuitOpen(_)
        | MediaClientError::PoolExhausted(_)
        | MediaClientError::TlsConfig(_) => DomainError::unavailable(e.to_string()),
    }
}
