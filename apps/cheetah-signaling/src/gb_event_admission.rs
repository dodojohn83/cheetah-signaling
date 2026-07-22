//! GB28181 event admission control.
//!
//! Classifies inbound `Gb28181Event`s by traffic class and priority, coalesces
//! redundant keepalive / position events, sheds low-priority work when the
//! application sink is overloaded, and dead-letters high/normal-priority events
//! that cannot be immediately admitted so they can be redriven later.

use cheetah_gb28181_driver_tokio::sink::EventSink;
use cheetah_gb28181_module::Gb28181Event;
use cheetah_http_api::metrics::RequestMetrics;
use cheetah_http_api::state::ApiState;
use cheetah_signal_types::admission::{
    BacklogController, CoalesceDecision, Coalescer, DeadLetterEntry, DeadLetterQueue,
    DeadLetterReason, Priority, TrafficClass,
};
use cheetah_signal_types::{Clock, GbMetricsRecorder, NodeId, TenantId};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::gb_catalog_buffer::{CATALOG_CLEANUP_INTERVAL, CatalogBuffer, RecordInfoBuffer};
use crate::gb_event_processing::process_event;

/// Interval between periodic dead-letter redrive attempts.
const REDRIVE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Maximum number of events to redrive in a single batch.
const REDRIVE_BATCH_SIZE: usize = 64;

/// Maximum redrive attempts for a dead-lettered event before it is dropped.
const MAX_REDRIVE_ATTEMPTS: u32 = 5;

/// Internal wrapper that carries an event together with its admission
/// classification so the worker does not need to re-classify.
#[derive(Clone, Debug)]
struct TaggedGb28181Event {
    event: Gb28181Event,
    traffic_class: TrafficClass,
    coalescing_key: Option<String>,
}

/// Shared admission state between the sink and the background worker.
struct AdmissionState {
    tx: mpsc::Sender<TaggedGb28181Event>,
    coalescer: Coalescer<String>,
    backlog: BacklogController,
    dlq: DeadLetterQueue<Gb28181Event>,
    /// Approximate number of events currently occupying the bounded channel.
    pending: usize,
    metrics: Arc<RequestMetrics>,
    tenant_id: Option<TenantId>,
}

/// Non-blocking event sink that forwards `Gb28181Event`s to a background
/// worker for processing through the application service layer.
pub struct GbApplicationEventSink {
    state: Arc<Mutex<AdmissionState>>,
    clock: Arc<dyn Clock>,
}

impl Clone for GbApplicationEventSink {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            clock: self.clock.clone(),
        }
    }
}

impl std::fmt::Debug for GbApplicationEventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GbApplicationEventSink")
            .finish_non_exhaustive()
    }
}

impl EventSink<Gb28181Event> for GbApplicationEventSink {
    fn emit(&self, event: Gb28181Event) {
        let now_ms = self.clock.now_monotonic().as_millis();
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.admit(event, now_ms);
    }
}

/// Spawns a background worker that consumes GB28181 events and applies them
/// through `DeviceService` using bounded in-memory queueing with admission
/// control. Returns the sink to be given to the UDP driver and a handle to
/// the spawned worker.
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
    cancel: CancellationToken,
) -> (
    Arc<dyn EventSink<Gb28181Event>>,
    tokio::task::JoinHandle<()>,
) {
    let queue_depth = queue_depth.max(1);
    let (tx, mut rx) = mpsc::channel(queue_depth);
    let metrics = state.metrics.clone();
    let clock = state.clock.clone();

    let high_watermark = (queue_depth as u64 * 4 / 5).max(1);
    let low_watermark = high_watermark / 2;
    let coalescer = Coalescer::new(queue_depth.max(256));
    let dlq_capacity = queue_depth.saturating_mul(2).max(256);
    let backlog = BacklogController::new(high_watermark, low_watermark);

    let admission = Arc::new(Mutex::new(AdmissionState {
        tx: tx.clone(),
        coalescer,
        backlog,
        dlq: DeadLetterQueue::new(dlq_capacity),
        pending: 0,
        metrics: metrics.clone(),
        tenant_id,
    }));

    let sink = Arc::new(GbApplicationEventSink {
        state: admission.clone(),
        clock: clock.clone(),
    }) as Arc<dyn EventSink<Gb28181Event>>;

    let mut catalog_buffer = CatalogBuffer::new(catalog_max_entries, catalog_max_items);
    let mut record_buffer = RecordInfoBuffer::new(record_max_entries, record_max_items);
    let mut cleanup = interval(CATALOG_CLEANUP_INTERVAL);
    cleanup.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut redrive = interval(REDRIVE_INTERVAL);
    redrive.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let batch_size = REDRIVE_BATCH_SIZE.min(queue_depth);

    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = cleanup.tick() => {
                    catalog_buffer.evict();
                    record_buffer.evict();
                    continue;
                }
                _ = redrive.tick() => {
                    let now_ms = clock.now_monotonic().as_millis();
                    if let Ok(mut guard) = admission.lock() {
                        guard.redrive(now_ms, batch_size);
                    }
                    continue;
                }
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(tagged) => {
                            {
                                let now_ms = clock.now_monotonic().as_millis();
                                if let Ok(mut guard) = admission.lock() {
                                    guard.pending = guard.pending.saturating_sub(1);
                                    guard.redrive(now_ms, batch_size);
                                }
                            }
                            if let Err(e) = process_event(
                                &state,
                                node_id,
                                tenant_id,
                                tagged.event,
                                &mut catalog_buffer,
                                &mut record_buffer,
                                gb_metrics.as_ref(),
                            ).await {
                                warn!(error = %e, "failed to process gb28181 event");
                            }
                            {
                                let now_ms = clock.now_monotonic().as_millis();
                                if let Ok(mut guard) = admission.lock() {
                                    if let Some(key) = tagged.coalescing_key {
                                        guard.coalescer.release(&key);
                                    }
                                    guard.redrive(now_ms, batch_size);
                                }
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

impl AdmissionState {
    /// Applies admission control to a single inbound event.
    fn admit(&mut self, event: Gb28181Event, now_ms: i64) {
        let (class, key) = classify_event(&event, self.tenant_id);
        let priority = class.priority();

        if let Some(ref key) = key
            && self.coalescer.observe(key.clone()) == CoalesceDecision::Coalesced
        {
            self.metrics.record_gb28181_event_coalesced();
            return;
        }

        let _observation = self.backlog.observe(self.pending as u64);
        if self.backlog.shed_low_priority() && priority == Priority::Low {
            self.shed_event(key);
            return;
        }

        let tagged = TaggedGb28181Event {
            event,
            traffic_class: class,
            coalescing_key: key,
        };
        match self.tx.try_send(tagged) {
            Ok(()) => {
                self.pending += 1;
                self.metrics.record_gb28181_event_admitted();
            }
            Err(e) => {
                let tagged = e.into_inner();
                if let Some(ref key) = tagged.coalescing_key {
                    self.coalescer.release(key);
                }
                if priority == Priority::Low {
                    self.shed_event(tagged.coalescing_key);
                    // `tagged.event` is dropped; do not dead-letter low-priority work.
                    let _ = tagged.event;
                } else {
                    self.dlq_push(tagged.event, DeadLetterReason::Overloaded, now_ms);
                }
            }
        }
    }

    /// Records a shed low-priority event and releases its coalescing key.
    fn shed_event(&mut self, key: Option<String>) {
        if let Some(key) = key {
            self.coalescer.release(&key);
        }
        self.metrics.record_gb28181_event_shed();
        self.metrics.record_gb28181_event_dropped();
    }

    /// Places an event in the dead-letter queue and records metrics, accounting
    /// for queue overflow displacing the oldest entry.
    fn dlq_push(&mut self, event: Gb28181Event, reason: DeadLetterReason, now_ms: i64) {
        let dropped_before = self.dlq.dropped_total();
        self.dlq.push(event, reason, now_ms);
        self.metrics.record_gb28181_event_dead_lettered();
        if self.dlq.dropped_total() > dropped_before {
            self.metrics.record_gb28181_event_dropped();
        }
    }

    /// Attempts to redrive up to `batch_size` dead-lettered events into the
    /// bounded channel.
    fn redrive(&mut self, _now_ms: i64, batch_size: usize) {
        let entries = self.dlq.drain(batch_size);
        for mut entry in entries {
            if entry.attempts >= MAX_REDRIVE_ATTEMPTS {
                self.drop_dead_letter_entry(entry);
                continue;
            }

            entry.attempts += 1;
            let (class, key) = classify_event(&entry.payload, self.tenant_id);
            let tagged = TaggedGb28181Event {
                event: entry.payload,
                traffic_class: class,
                coalescing_key: key,
            };

            match self.tx.try_send(tagged) {
                Ok(()) => {
                    self.pending += 1;
                    self.metrics.record_gb28181_event_admitted();
                    self.metrics.record_gb28181_event_redriven();
                }
                Err(e) => {
                    let tagged = e.into_inner();
                    if entry.attempts >= MAX_REDRIVE_ATTEMPTS {
                        self.drop_dead_letter_tagged(tagged, entry.attempts);
                    } else {
                        entry.payload = tagged.event;
                        self.dlq.push_entry(entry);
                    }
                }
            }
        }
    }

    /// Drops a dead-letter entry that has exhausted its redrive budget.
    fn drop_dead_letter_entry(&mut self, entry: DeadLetterEntry<Gb28181Event>) {
        let (class, key) = classify_event(&entry.payload, self.tenant_id);
        self.drop_dead_letter_inner(class, key, entry.attempts);
    }

    fn drop_dead_letter_tagged(&mut self, tagged: TaggedGb28181Event, attempts: u32) {
        self.drop_dead_letter_inner(tagged.traffic_class, tagged.coalescing_key, attempts);
    }

    fn drop_dead_letter_inner(&mut self, class: TrafficClass, key: Option<String>, attempts: u32) {
        if let Some(key) = key {
            self.coalescer.release(&key);
        }
        self.metrics.record_gb28181_event_redrive_exhausted();
        self.metrics.record_gb28181_event_dropped();
        warn!(
            event_type = ?class,
            attempts,
            "gb28181 event dead-letter redrive budget exhausted; dropping event"
        );
    }
}

/// Classifies a `Gb28181Event` into a traffic class and, when the class is
/// coalescible, a stable `tenant:device:event-type` key.
fn classify_event(
    event: &Gb28181Event,
    tenant_id: Option<TenantId>,
) -> (TrafficClass, Option<String>) {
    let class = match event {
        Gb28181Event::DeviceControlResponseReceived { .. }
        | Gb28181Event::MediaSessionStarted { .. }
        | Gb28181Event::MediaSessionStopped { .. }
        | Gb28181Event::MediaSessionFailed { .. }
        | Gb28181Event::CascadePlayRequested { .. }
        | Gb28181Event::CascadePlayStopped { .. } => TrafficClass::Command,
        Gb28181Event::CatalogReceived { .. } => TrafficClass::Catalog,
        Gb28181Event::AlarmReceived { .. } => TrafficClass::Alarm,
        Gb28181Event::Keepalive { .. } => TrafficClass::Keepalive,
        Gb28181Event::MobilePositionReceived { .. } => TrafficClass::Position,
        Gb28181Event::RecordInfoReceived { .. } => TrafficClass::Other,
        _ => TrafficClass::Other,
    };

    let key = if class.is_coalescible() {
        coalescing_key(event, tenant_id)
    } else {
        None
    };

    (class, key)
}

/// Builds a `tenant:device:event-type` coalescing key for keepalive and
/// position events.
fn coalescing_key(event: &Gb28181Event, tenant_id: Option<TenantId>) -> Option<String> {
    let tenant = tenant_id?;
    let (device_id, class_label) = match event {
        Gb28181Event::Keepalive { device_id, .. } => {
            (device_id.as_ref(), TrafficClass::Keepalive.as_str())
        }
        Gb28181Event::MobilePositionReceived { device_id, .. } => {
            (device_id.as_ref(), TrafficClass::Position.as_str())
        }
        _ => return None,
    };
    Some(format!("{}:{}:{}", tenant, device_id, class_label))
}
