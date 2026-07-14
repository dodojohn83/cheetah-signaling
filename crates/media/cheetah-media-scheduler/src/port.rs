//! Domain `MediaPort` implementation backed by the media scheduler.

use crate::scheduler::MediaScheduler;
use cheetah_domain::{DomainError, MediaPort, MediaRequirements, MediaReservation};
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, MediaBindingId, MediaSessionId, TenantId, UtcTimestamp,
};
use std::sync::Arc;

use crate::error::SchedulerError;

/// A `MediaPort` that selects media nodes using the scheduler.
#[derive(Clone)]
pub struct SchedulerMediaPort {
    scheduler: Arc<dyn MediaScheduler>,
}

impl SchedulerMediaPort {
    /// Creates a new scheduler-backed media port.
    pub fn new(scheduler: Arc<dyn MediaScheduler>) -> Self {
        Self { scheduler }
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
            .map_err(map_scheduler_error)?;
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
