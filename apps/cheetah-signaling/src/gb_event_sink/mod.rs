//! GB28181 application event sink.
//!
//! Routes incoming GB28181 driver events into the application service layer
//! instead of only logging them. The sink is non-blocking: it drops events
//! when the bounded channel is full and processes them asynchronously in a
//! background worker.
//!
//! The implementation is split into cohesive submodules:
//! - [`dispatch`]: the `Gb28181Event` match/dispatch and request-context helpers;
//! - [`device`]: device registration/presence and bootstrap query helpers;
//! - [`catalog`]: channel-catalog replacement and channel metadata construction;
//! - [`media_session`]: media-session lifecycle transitions;
//! - [`outbox`]: `Gb28181EventReceived` outbox envelope helpers.

mod catalog;
mod device;
mod dispatch;
mod media_session;
mod outbox;

use cheetah_gb28181_driver_tokio::sink::EventSink;
use cheetah_gb28181_module::Gb28181Event;
use cheetah_http_api::metrics::RequestMetrics;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::{GbMetricsRecorder, NodeId, SignalError, SignalErrorKind, TenantId};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::gb_catalog_buffer::{CATALOG_CLEANUP_INTERVAL, CatalogBuffer, RecordInfoBuffer};
use dispatch::process_event;

/// Non-blocking event sink that forwards `Gb28181Event`s to a background
/// worker for processing through the application service layer.
#[derive(Clone, Debug)]
pub struct GbApplicationEventSink {
    tx: mpsc::Sender<Gb28181Event>,
    metrics: Arc<RequestMetrics>,
}

impl EventSink<Gb28181Event> for GbApplicationEventSink {
    fn emit(&self, event: Gb28181Event) {
        if let Err(e) = self.tx.try_send(event) {
            self.metrics.record_gb28181_event_dropped();
            warn!(error = %e, "gb28181 event sink full; dropping event");
        }
    }
}

/// Spawns a background worker that consumes GB28181 events and applies them
/// through `DeviceService` using bounded in-memory queueing. Returns the sink
/// to be given to the UDP driver and a handle to the spawned worker.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    state: ApiState,
    node_id: NodeId,
    tenant_id: Option<TenantId>,
    queue_depth: usize,
    catalog_max_entries: usize,
    catalog_max_items: usize,
    record_max_entries: usize,
    record_max_items: usize,
    gb_metrics: Arc<dyn GbMetricsRecorder>,
    cancel: tokio_util::sync::CancellationToken,
) -> (
    Arc<dyn EventSink<Gb28181Event>>,
    tokio::task::JoinHandle<()>,
) {
    let queue_depth = queue_depth.max(1);
    let (tx, mut rx) = mpsc::channel(queue_depth);
    let metrics = state.metrics.clone();
    let sink = Arc::new(GbApplicationEventSink { tx, metrics }) as Arc<dyn EventSink<Gb28181Event>>;
    let mut catalog_buffer = CatalogBuffer::new(catalog_max_entries, catalog_max_items);
    let mut record_buffer = RecordInfoBuffer::new(record_max_entries, record_max_items);
    let mut cleanup = tokio::time::interval(CATALOG_CLEANUP_INTERVAL);
    cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = cleanup.tick() => {
                    catalog_buffer.evict();
                    record_buffer.evict();
                    continue;
                }
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(event) => {
                            if let Err(e) = process_event(&state, node_id, tenant_id, event, &mut catalog_buffer, &mut record_buffer, gb_metrics.as_ref()).await {
                                warn!(error = %e, "failed to process gb28181 event");
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        info!("gb28181 application event sink stopped");
    });
    (sink, handle)
}

/// Maps a storage-layer transaction error to a generic internal `SignalError`.
pub(super) fn storage_error(e: cheetah_storage_api::StorageError) -> SignalError {
    SignalError::new(
        SignalErrorKind::Internal,
        format!("failed to begin storage transaction: {e}"),
    )
}
