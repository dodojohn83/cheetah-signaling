//! Domain `MediaPort` implementation backed by the media scheduler.

use crate::metrics::MediaMetrics;
use crate::scheduler::MediaScheduler;
use cheetah_domain::{
    DomainError, MediaClient, MediaNodeCommand, MediaNodeCommandResult, MediaNodeSessionRef,
    MediaPort, MediaRequirements, MediaReservation,
};
use cheetah_signal_types::Page;
use cheetah_signal_types::{
    ChannelId, Clock, DeviceId, MediaBindingId, MediaSessionId, MetricsExporter, NodeId,
    PageRequest, TenantId, UtcTimestamp,
};
use std::sync::Arc;
use std::time::Instant;

use crate::error::SchedulerError;

/// A `MediaPort` that selects media nodes using the scheduler and dispatches
/// commands to them via the media control client.
#[derive(Clone)]
pub struct SchedulerMediaPort {
    scheduler: Arc<dyn MediaScheduler>,
    client: Arc<dyn MediaClient>,
    metrics: Arc<MediaMetrics>,
}

impl SchedulerMediaPort {
    /// Creates a new scheduler-backed media port.
    pub fn new(
        scheduler: Arc<dyn MediaScheduler>,
        client: Arc<dyn MediaClient>,
        metrics: Arc<MediaMetrics>,
    ) -> Self {
        Self {
            scheduler,
            client,
            metrics,
        }
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
            &self.metrics,
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
            &self.metrics,
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
            &self.metrics,
        )
        .await
    }

    async fn release(
        &self,
        tenant_id: TenantId,
        media_binding_id: MediaBindingId,
        clock: &dyn Clock,
    ) -> Result<(), DomainError> {
        match self
            .scheduler
            .release(tenant_id, media_binding_id, clock)
            .await
        {
            Ok(()) => Ok(()),
            Err(SchedulerError::ReservationNotFound { .. }) => {
                tracing::info!(
                    %tenant_id,
                    %media_binding_id,
                    "release ignored missing reservation; treating as already released"
                );
                Ok(())
            }
            Err(e) => Err(map_scheduler_error(e)),
        }
    }

    async fn execute(
        &self,
        mut command: MediaNodeCommand,
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

        if command.owner_epoch.0 == 0 {
            return Err(DomainError::invalid_argument("owner_epoch is required"));
        }

        if let Some(deadline) = command.deadline
            && deadline.is_elapsed(clock.now_wall())
        {
            return Err(DomainError::unavailable("command deadline elapsed"));
        }

        let endpoint = node.control_endpoint.clone();
        if command.contract_version == 0 {
            command.contract_version = node.contract_version;
        }

        let start = Instant::now();
        let result = self.client.execute(&endpoint, &command).await;
        let duration = start.elapsed();
        match &result {
            Ok(_) => self.metrics.record_rpc(duration, false),
            Err(_) => self.metrics.record_rpc(duration, true),
        }
        result
    }

    async fn list_nodes(
        &self,
        _tenant_id: TenantId,
        clock: &dyn Clock,
    ) -> Result<Vec<cheetah_domain::MediaNode>, DomainError> {
        let nodes = self.scheduler.list_nodes(clock).await;
        self.metrics.record_node_snapshot(&nodes, clock);
        Ok(nodes)
    }

    async fn get_node(
        &self,
        node_id: NodeId,
        clock: &dyn Clock,
    ) -> Result<Option<cheetah_domain::MediaNode>, DomainError> {
        Ok(self.scheduler.get_node(node_id, clock).await)
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

        let endpoint = node.control_endpoint.clone();
        let start = Instant::now();
        let result = self
            .client
            .list_sessions(
                &endpoint,
                tenant_id,
                media_node_id,
                node.instance_epoch_value(),
                page,
            )
            .await;
        let duration = start.elapsed();
        match &result {
            Ok(_) => self.metrics.record_rpc(duration, false),
            Err(_) => self.metrics.record_rpc(duration, true),
        }
        result
    }

    fn metrics(&self) -> Option<Arc<dyn MetricsExporter>> {
        Some(self.metrics.clone())
    }

    fn record_reconcile(
        &self,
        nodes_scanned: u64,
        sessions_repaired: u64,
        sessions_failed: u64,
        orphans_cleaned: u64,
    ) {
        self.metrics.record_reconcile(
            nodes_scanned,
            sessions_repaired,
            sessions_failed,
            orphans_cleaned,
        );
    }

    fn record_forced_cleanup(&self, count: u64) {
        self.metrics.record_forced_cleanup(count);
    }

    async fn drain_node(
        &self,
        tenant_id: TenantId,
        node_id: NodeId,
        clock: &dyn Clock,
    ) -> Result<(), DomainError> {
        self.scheduler
            .drain(node_id, tenant_id, clock)
            .await
            .map_err(map_scheduler_error)?;
        Ok(())
    }
}

async fn reserve(
    scheduler: Arc<dyn MediaScheduler>,
    tenant_id: TenantId,
    binding_id: MediaBindingId,
    requirements: &MediaRequirements,
    clock: &dyn Clock,
    metrics: &MediaMetrics,
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
                    metrics.record_reservation(false, Some("unsupported_talk"));
                    DomainError::not_supported("talk not supported by any media node")
                } else {
                    let reason = schedule_error_reason(&e);
                    metrics.record_reservation(false, Some(reason));
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
                metrics.record_reservation(true, None);
                return Ok(MediaReservation {
                    media_node_id: node_id,
                    media_node_instance_epoch: instance_epoch,
                    contract_version: node.contract_version,
                });
            }
            // The scheduler returned a node, but it drained, left or ran out of
            // capacity between scheduling and reservation. Exclude it and try
            // the next best candidate rather than failing the whole request.
            Err(SchedulerError::CapacityExhausted(_))
            | Err(SchedulerError::NodeDraining(_))
            | Err(SchedulerError::NodeNotFound(_)) => {
                excluded.push(node_id);
                continue;
            }
            Err(e) => {
                let reason = scheduler_reserve_error_reason(&e);
                metrics.record_reservation(false, Some(reason));
                return Err(map_scheduler_error(e));
            }
        }
    }

    metrics.record_reservation(false, Some("no_capacity"));
    Err(DomainError::unavailable(
        "no media node had capacity after retries",
    ))
}

fn map_scheduler_error(e: crate::error::SchedulerError) -> DomainError {
    match e {
        crate::error::SchedulerError::Domain(err) => err,
        crate::error::SchedulerError::EventStream(_) | crate::error::SchedulerError::Backend(_) => {
            DomainError::unavailable(e.to_string())
        }
        crate::error::SchedulerError::NoNode(_)
        | crate::error::SchedulerError::CapacityExhausted(_)
        | crate::error::SchedulerError::NodeDraining(_) => DomainError::unavailable(e.to_string()),
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

fn schedule_error_reason(e: &crate::error::SchedulerError) -> &'static str {
    match e {
        crate::error::SchedulerError::NoNode(_) => "no_node",
        crate::error::SchedulerError::Backend(_) | crate::error::SchedulerError::EventStream(_) => {
            "internal"
        }
        _ => "internal",
    }
}

fn scheduler_reserve_error_reason(e: &crate::error::SchedulerError) -> &'static str {
    match e {
        crate::error::SchedulerError::CapacityExhausted(_) => "capacity_exhausted",
        crate::error::SchedulerError::NodeNotFound(_)
        | crate::error::SchedulerError::ReservationNotFound { .. } => "not_found",
        crate::error::SchedulerError::InvalidArgument(_)
        | crate::error::SchedulerError::IdentityMismatch { .. } => "invalid_argument",
        crate::error::SchedulerError::Domain(_) => "domain",
        crate::error::SchedulerError::Backend(_) | crate::error::SchedulerError::EventStream(_) => {
            "internal"
        }
        _ => "internal",
    }
}
