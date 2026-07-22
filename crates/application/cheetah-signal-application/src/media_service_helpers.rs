//! Shared helpers for media service event persistence.

use crate::MediaService;
use crate::media_service::{media_binding_resource_ref, media_session_resource_ref, wrap_event};
use cheetah_domain::{DomainError, MediaBinding, MediaSession, UnitOfWork};
use cheetah_signal_types::RequestContext;

/// Appends a media session domain event to the outbox.
pub(crate) async fn append_session_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    session: &MediaSession,
    event: cheetah_domain::DomainEvent,
) -> Result<(), DomainError> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_session_resource_ref(context.tenant_id, session.media_session_id()),
            session.revision().0,
            event,
        ))
        .await?;
    Ok(())
}

/// Appends a media binding domain event to the outbox.
pub(crate) async fn append_binding_event(
    service: &MediaService,
    context: &RequestContext,
    uow: &mut dyn UnitOfWork,
    binding: &MediaBinding,
    event: cheetah_domain::DomainEvent,
) -> Result<(), DomainError> {
    uow.outbox()
        .append(wrap_event(
            service.id_generator.as_ref(),
            service.clock.as_ref(),
            context,
            context.tenant_id,
            media_binding_resource_ref(context.tenant_id, binding.media_binding_id()),
            binding.revision().0,
            event,
        ))
        .await?;
    Ok(())
}
