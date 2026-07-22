//! Media application service.

use crate::dto::{ControlPlaybackRequest, MediaSessionDto, OperationDto, StopLiveRequest};
use cheetah_domain::{
    Channel, ChannelStatus, CommandPayload, Device, DeviceLifecycle, DeviceOwnerResolver,
    DomainError, DomainEvent, IdempotencyScope, MediaPort, MediaPurpose, MediaRequirements,
    MediaSessionState, Operation, OperationResult, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Clock, Deadline, DeviceId, Event, IdGenerator, MediaBindingId, MediaSessionId,
    NodeId, OperationId, RequestContext, ResourceId, ResourceKind, ResourceRef, TenantId,
    UtcTimestamp,
};

/// Application service for media lifecycle.
#[derive(Clone)]
pub struct MediaService {
    pub(crate) clock: std::sync::Arc<dyn Clock>,
    pub(crate) id_generator: std::sync::Arc<dyn IdGenerator>,
    pub(crate) owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
    pub(crate) media_port: std::sync::Arc<dyn MediaPort>,
    pub(crate) source_node_id: NodeId,
    /// Grace period after a binding is marked `NeedsVerification` before the
    /// reconciler escalates to `migrate_or_fail` (milliseconds).
    pub(crate) needs_verification_grace_ms: u64,
}

impl MediaService {
    /// Default grace period for `NeedsVerification` escalation.
    pub const DEFAULT_NEEDS_VERIFICATION_GRACE_MS: u64 = 60_000;

    /// Creates a new media service.
    pub fn new(
        clock: std::sync::Arc<dyn Clock>,
        id_generator: std::sync::Arc<dyn IdGenerator>,
        owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
        media_port: std::sync::Arc<dyn MediaPort>,
        source_node_id: NodeId,
    ) -> Self {
        Self {
            clock,
            id_generator,
            owner_resolver,
            media_port,
            source_node_id,
            needs_verification_grace_ms: Self::DEFAULT_NEEDS_VERIFICATION_GRACE_MS,
        }
    }

    /// Sets the grace period before a `NeedsVerification` binding is escalated to
    /// migration or failure.
    pub fn set_needs_verification_grace_ms(&mut self, ms: u64) {
        self.needs_verification_grace_ms = ms.max(1_000);
    }

    /// Lists media nodes reachable through the configured media port.
    pub async fn list_media_nodes(
        &self,
        context: &RequestContext,
    ) -> crate::Result<Vec<cheetah_domain::MediaNode>> {
        self.media_port
            .list_nodes(context.tenant_id, self.clock.as_ref())
            .await
            .map_err(crate::SignalError::from)
    }

    /// Marks the given media node as draining.
    pub async fn drain_media_node(
        &self,
        context: &RequestContext,
        node_id: NodeId,
    ) -> crate::Result<()> {
        self.media_port
            .drain_node(context.tenant_id, node_id, self.clock.as_ref())
            .await
            .map_err(crate::SignalError::from)
    }

    /// Stops a media session (live, playback or talk).
    pub async fn stop_live(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: StopLiveRequest,
    ) -> crate::Result<MediaSessionDto> {
        let tenant_id = context.tenant_id;
        let media_session_id = request.media_session_id.parse::<MediaSessionId>()?;

        let target = media_session_resource_ref(tenant_id, media_session_id);
        let scope = IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target,
            request.idempotency_key,
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .operation_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            let session = uow
                .media_session_repository()
                .get(tenant_id, media_session_id)
                .await?
                .ok_or_else(|| {
                    crate::SignalError::from(DomainError::not_found(
                        "media session",
                        media_session_id.to_string(),
                    ))
                })?;
            // Re-emit the session state from the existing operation so the
            // caller sees the idempotent result.
            let _ = existing;
            return Ok(MediaSessionDto::from(&session));
        }

        let mut session = uow
            .media_session_repository()
            .get(tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media session",
                    media_session_id.to_string(),
                ))
            })?;

        let payload = CommandPayload::StopMediaSession { media_session_id };
        let (mut operation, op_event) = Operation::new(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            scope.idempotency_key,
            session.device_id(),
            scope.target,
            payload,
            None,
            session.owner_epoch(),
        )
        .map_err(crate::SignalError::from)?;

        if session.is_terminal() {
            // The session is already stopped/failed: create a terminal stop
            // operation as compensation and return the existing session.
            let submitted_revision = operation.revision().0;
            let op_started_event = operation
                .start(self.clock.as_ref())
                .map_err(crate::SignalError::from)?;
            let started_revision = operation.revision().0;
            let op_complete_event = operation
                .complete(OperationResult::success(), self.clock.as_ref())
                .map_err(crate::SignalError::from)?;
            let completed_revision = operation.revision().0;

            uow.operation_repository().save(&operation).await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    submitted_revision,
                    op_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    started_revision,
                    op_started_event,
                ))
                .await?;
            uow.outbox()
                .append(wrap_event(
                    self.id_generator.as_ref(),
                    self.clock.as_ref(),
                    context,
                    tenant_id,
                    operation_resource_ref(tenant_id, operation.operation_id()),
                    completed_revision,
                    op_complete_event,
                ))
                .await?;
            uow.commit().await?;
            return Ok(MediaSessionDto::from(&session));
        }

        let binding = uow
            .media_binding_repository()
            .get_by_media_session(tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media binding",
                    media_session_id.to_string(),
                ))
            })?;

        // Mark the session as stopping before the media-node RPC so new
        // start/control commands on the same session are rejected.
        let stop_event = session
            .stop(self.clock.as_ref())
            .map_err(crate::SignalError::from)?;

        uow.operation_repository().save(&operation).await?;
        uow.media_session_repository().save(&session).await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                operation_resource_ref(tenant_id, operation.operation_id()),
                operation.revision().0,
                op_event,
            ))
            .await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                media_session_resource_ref(tenant_id, session.media_session_id()),
                session.revision().0,
                stop_event,
            ))
            .await?;

        uow.commit().await?;

        self.dispatch_stop_command(
            context,
            uow,
            operation.operation_id(),
            session.media_session_id(),
            binding.media_binding_id(),
            binding.media_node_id(),
            binding.media_node_instance_epoch(),
            0,
            binding.owner_epoch(),
            operation.deadline(),
            operation.idempotency_scope().idempotency_key.clone(),
        )
        .await
    }

    /// Controls an active playback session.
    pub async fn control_playback(
        &self,
        context: &RequestContext,
        uow: &mut dyn UnitOfWork,
        request: ControlPlaybackRequest,
    ) -> crate::Result<OperationDto> {
        let tenant_id = context.tenant_id;
        let media_session_id = request.media_session_id.parse::<MediaSessionId>()?;

        let target = media_session_resource_ref(tenant_id, media_session_id);
        let scope = IdempotencyScope::new(
            tenant_id,
            context.principal.id.clone(),
            target,
            request.idempotency_key,
        )
        .map_err(crate::SignalError::from)?;

        if let Some(existing) = uow
            .operation_repository()
            .get_by_idempotency(&scope)
            .await?
        {
            return Ok(OperationDto::from(&existing));
        }

        let session = uow
            .media_session_repository()
            .get(tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media session",
                    media_session_id.to_string(),
                ))
            })?;

        if session.state() != MediaSessionState::Active
            || session.purpose() != MediaPurpose::Playback
        {
            return Err(crate::SignalError::from(DomainError::invalid_argument(
                "media session must be active playback",
            )));
        }

        let binding = uow
            .media_binding_repository()
            .get_by_media_session(tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media binding",
                    media_session_id.to_string(),
                ))
            })?;

        let payload = CommandPayload::ControlPlayback {
            media_session_id,
            command: request.command.into(),
        };
        let (operation, op_event) = Operation::new(
            self.id_generator.as_ref(),
            self.clock.as_ref(),
            context,
            scope.idempotency_key,
            session.device_id(),
            scope.target,
            payload,
            None,
            session.owner_epoch(),
        )
        .map_err(crate::SignalError::from)?;

        uow.operation_repository().save(&operation).await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                operation_resource_ref(tenant_id, operation.operation_id()),
                operation.revision().0,
                op_event,
            ))
            .await?;

        uow.commit().await?;

        self.dispatch_control_command(
            context,
            uow,
            operation.operation_id(),
            session.media_session_id(),
            binding.media_binding_id(),
            binding.media_node_id(),
            binding.media_node_instance_epoch(),
            0,
            binding.owner_epoch(),
            operation.deadline(),
            operation.idempotency_scope().idempotency_key.clone(),
            operation.command().payload().clone(),
        )
        .await
    }

    pub(crate) async fn ensure_device_and_channel_ready(
        &self,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> crate::Result<(Device, Channel)> {
        let device = uow
            .device_repository()
            .get(tenant_id, device_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found("device", device_id.to_string()))
            })?;
        if device.lifecycle() != DeviceLifecycle::Active {
            return Err(crate::SignalError::from(DomainError::unavailable(
                "device is not active",
            )));
        }

        let channel = uow
            .channel_repository()
            .get(tenant_id, device_id, channel_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found("channel", channel_id.to_string()))
            })?;
        if !channel.enabled() || channel.status() == ChannelStatus::Unknown {
            return Err(crate::SignalError::from(DomainError::unavailable(
                "channel is not ready",
            )));
        }
        Ok((device, channel))
    }
}

pub(crate) fn build_media_requirements(
    device: &Device,
    channel: &Channel,
    purpose: MediaPurpose,
    media_session_id: MediaSessionId,
    extra_constraints: std::collections::BTreeMap<String, String>,
) -> MediaRequirements {
    let mut codecs = Vec::new();
    for profile in channel.stream_profiles() {
        if !profile.encoding.is_empty() && !codecs.contains(&profile.encoding) {
            codecs.push(profile.encoding.clone());
        }
    }
    MediaRequirements {
        protocol: device.protocol().to_string(),
        operation: purpose.to_string(),
        session_type: purpose.to_string(),
        transport: None,
        encapsulation: None,
        codecs,
        zone: None,
        network_zone: None,
        tenant_constraints: std::collections::BTreeMap::new(),
        required_constraints: extra_constraints,
        media_session_id: Some(media_session_id.to_string()),
        require_media_sender: purpose.requires_media_sender(),
        // No contract-version pin yet; the device/channel negotiation does not
        // yet supply a target version. The scheduler treats 0 as "no requirement".
        contract_version: 0,
    }
}

impl std::fmt::Debug for MediaService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaService").finish_non_exhaustive()
    }
}

pub(crate) fn parse_deadline(deadline: Option<String>) -> crate::Result<Option<Deadline>> {
    match deadline {
        None => Ok(None),
        Some(s) => {
            let ts = s.parse::<UtcTimestamp>()?;
            Ok(Some(Deadline::from_timestamp(ts)))
        }
    }
}

pub(crate) fn wrap_event(
    id_generator: &dyn IdGenerator,
    clock: &dyn Clock,
    context: &RequestContext,
    tenant_id: TenantId,
    aggregate_ref: ResourceRef,
    aggregate_sequence: u64,
    payload: DomainEvent,
) -> Event<DomainEvent> {
    Event::new(
        id_generator,
        clock,
        context,
        tenant_id,
        aggregate_ref,
        aggregate_sequence,
        payload,
    )
}

pub(crate) fn channel_resource_ref(tenant_id: TenantId, channel_id: ChannelId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Channel,
        id: ResourceId::Channel(channel_id),
    }
}

pub(crate) fn media_session_resource_ref(
    tenant_id: TenantId,
    media_session_id: MediaSessionId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::MediaSession,
        id: ResourceId::MediaSession(media_session_id),
    }
}

pub(crate) fn media_binding_resource_ref(
    tenant_id: TenantId,
    media_binding_id: MediaBindingId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::MediaBinding,
        id: ResourceId::MediaBinding(media_binding_id),
    }
}

pub(crate) fn operation_resource_ref(
    tenant_id: TenantId,
    operation_id: OperationId,
) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Operation,
        id: ResourceId::Operation(operation_id),
    }
}
