//! Media application service.

use crate::dto::{ControlPlaybackRequest, MediaSessionDto, OperationDto, StopLiveRequest};
use cheetah_domain::{
    ChannelStatus, CommandPayload, DeviceLifecycle, DeviceOwnerResolver, DomainError, DomainEvent,
    IdempotencyScope, MediaPort, MediaPurpose, MediaSessionState, Operation, UnitOfWork,
};
use cheetah_signal_types::{
    ChannelId, Clock, Deadline, DeviceId, Event, IdGenerator, MediaBindingId, MediaSessionId,
    OperationId, RequestContext, ResourceId, ResourceKind, ResourceRef, TenantId, UtcTimestamp,
};

/// Application service for media lifecycle.
#[derive(Clone)]
pub struct MediaService {
    pub(crate) clock: std::sync::Arc<dyn Clock>,
    pub(crate) id_generator: std::sync::Arc<dyn IdGenerator>,
    pub(crate) owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
    pub(crate) media_port: std::sync::Arc<dyn MediaPort>,
}

impl MediaService {
    /// Creates a new media service.
    pub fn new(
        clock: std::sync::Arc<dyn Clock>,
        id_generator: std::sync::Arc<dyn IdGenerator>,
        owner_resolver: std::sync::Arc<dyn DeviceOwnerResolver>,
        media_port: std::sync::Arc<dyn MediaPort>,
    ) -> Self {
        Self {
            clock,
            id_generator,
            owner_resolver,
            media_port,
        }
    }

    /// Stops a live media session.
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

        if uow
            .operation_repository()
            .get_by_idempotency(&scope)
            .await?
            .is_some()
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
        let mut binding = uow
            .media_binding_repository()
            .get_by_media_session(tenant_id, media_session_id)
            .await?
            .ok_or_else(|| {
                crate::SignalError::from(DomainError::not_found(
                    "media binding",
                    media_session_id.to_string(),
                ))
            })?;

        self.media_port
            .release(tenant_id, binding.media_binding_id())
            .await?;

        let session_event = session
            .stop(self.clock.as_ref())
            .map_err(crate::SignalError::from)?;
        let binding_event = binding
            .release(self.clock.as_ref())
            .map_err(crate::SignalError::from)?;

        uow.media_session_repository().save(&session).await?;
        uow.media_binding_repository().save(&binding).await?;

        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                media_session_resource_ref(tenant_id, session.media_session_id()),
                session.revision().0,
                session_event,
            ))
            .await?;
        uow.outbox()
            .append(wrap_event(
                self.id_generator.as_ref(),
                self.clock.as_ref(),
                context,
                tenant_id,
                media_binding_resource_ref(tenant_id, binding.media_binding_id()),
                binding.revision().0,
                binding_event,
            ))
            .await?;

        let payload = CommandPayload::StopMediaSession { media_session_id };
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
        Ok(MediaSessionDto::from(&session))
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
        Ok(OperationDto::from(&operation))
    }

    pub(crate) async fn ensure_device_and_channel_ready(
        &self,
        uow: &mut dyn UnitOfWork,
        tenant_id: TenantId,
        device_id: DeviceId,
        channel_id: ChannelId,
    ) -> crate::Result<()> {
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
        Ok(())
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
