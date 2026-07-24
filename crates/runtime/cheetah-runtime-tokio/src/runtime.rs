//! Runtime entry point: starts shard workers and a timer wheel.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cheetah_runtime_api::{
    AdmissionController as AdmissionControllerTrait, DeviceActor, RuntimeConfig, RuntimeError,
    RuntimeMessage, RuntimeMetrics, RuntimeMetricsSnapshot, Scheduler, SessionRegistry,
    ShardRouter,
};
use cheetah_signal_types::Clock;
use cheetah_signal_types::admission::TrafficClass;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::admission::AdmissionController;
use crate::admission_policy::{AdmissionOutcome, AdmissionPolicy, AdmissionTicket};
use crate::shard::{Shard, ShardConfig};
use crate::system_clock::SystemClock;
use crate::timer_scheduler::TimerScheduler;
use crate::timer_wheel::TimerWheel;

/// Interval between backlog polls while draining.
const DRAIN_POLL_INTERVAL_MS: u64 = 10;

/// Outcome of a bounded [`Runtime::drain`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainOutcome {
    /// `true` if all in-flight work finished before the deadline.
    pub drained_within_deadline: bool,
    /// Aggregate shard-mailbox backlog remaining when the drain finished.
    pub remaining_backlog: u64,
}

/// A runtime manages fixed shard workers and a timer wheel.
pub struct Runtime<A: DeviceActor> {
    inner: Arc<RuntimeInner<A>>,
}

struct RuntimeInner<A: DeviceActor> {
    config: RuntimeConfig,
    admission: AdmissionController,
    session_registry: SessionRegistry<A::SessionHandle>,
    metrics: Arc<RuntimeMetrics>,
    timer_shutdown_tx: mpsc::Sender<()>,
    join_handles: Mutex<Option<Vec<JoinHandle<()>>>>,
    draining: AtomicBool,
}

impl<A: DeviceActor> std::fmt::Debug for Runtime<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl<A: DeviceActor> Clone for Runtime<A> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<A: DeviceActor> Runtime<A> {
    /// Starts the runtime and returns the runtime plus an output receiver.
    pub fn start(
        config: RuntimeConfig,
    ) -> Result<(Runtime<A>, mpsc::Receiver<A::Output>), RuntimeError> {
        config.validate()?;

        let (output_tx, output_rx) = mpsc::channel(config.output_channel_capacity);
        let (timer_tx, timer_rx) = mpsc::channel(config.timer_command_channel_capacity);
        let (timer_shutdown_tx, timer_shutdown_rx) = mpsc::channel(1);

        let router = ShardRouter::new(config.shard_count);
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let scheduler: Arc<dyn Scheduler> = Arc::new(TimerScheduler::new(timer_tx));
        let id_gen = Arc::new(AtomicU64::new(1));
        let session_registry = SessionRegistry::new(config.max_sessions);
        let metrics = Arc::new(RuntimeMetrics::new());

        let mut shard_senders = Vec::with_capacity(config.shard_count);
        let mut join_handles = Vec::with_capacity(config.shard_count.saturating_add(1));

        for _ in 0..config.shard_count {
            let (sender, receiver) = mpsc::channel(config.shard_mailbox_capacity);
            shard_senders.push(sender);
            join_handles.push(tokio::spawn(Shard::run::<A>(
                ShardConfig::from(&config),
                receiver,
                output_tx.clone(),
                scheduler.clone(),
                clock.clone(),
                id_gen.clone(),
                session_registry.clone(),
                metrics.clone(),
            )));
        }

        let senders = Arc::new(shard_senders);
        let policy = Arc::new(AdmissionPolicy::new(&config.admission));
        let admission = AdmissionController::new(
            router.clone(),
            senders.clone(),
            config.shard_mailbox_capacity,
            metrics.clone(),
            clock.clone(),
            Some(policy),
        );

        join_handles.push(tokio::spawn(TimerWheel::run(
            timer_rx,
            timer_shutdown_rx,
            senders,
            router,
            config.timer_tick_resolution_ms,
            config.max_pending_dispatch,
            metrics.clone(),
        )));

        let inner = Arc::new(RuntimeInner {
            config,
            admission,
            session_registry,
            metrics,
            timer_shutdown_tx,
            join_handles: Mutex::new(Some(join_handles)),
            draining: AtomicBool::new(false),
        });

        Ok((Runtime { inner }, output_rx))
    }

    /// Sends a message to the runtime for processing by the appropriate shard.
    pub fn send_message(
        &self,
        device_key: cheetah_runtime_api::DeviceKey,
        message: RuntimeMessage,
    ) -> Result<(), RuntimeError> {
        if matches!(message, RuntimeMessage::Shutdown) {
            return Err(RuntimeError::InvalidArgument(
                "use shutdown() to stop the runtime".into(),
            ));
        }
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(RuntimeError::Draining);
        }
        self.inner.admission.send(device_key, message)
    }

    /// Admits a classified message through the full admission policy: per-source
    /// and per-method rate limiting, priority-aware overload shedding,
    /// coalescing of redundant keepalive/position events and bounded
    /// dead-lettering.
    ///
    /// New work is rejected with [`RuntimeError::Draining`] once [`Runtime::drain`]
    /// has been invoked.
    pub fn admit(
        &self,
        ticket: AdmissionTicket,
        message: RuntimeMessage,
    ) -> Result<AdmissionOutcome, RuntimeError> {
        if matches!(message, RuntimeMessage::Shutdown) {
            return Err(RuntimeError::InvalidArgument(
                "use shutdown() to stop the runtime".into(),
            ));
        }
        if self.inner.draining.load(Ordering::Acquire) {
            return Err(RuntimeError::Draining);
        }
        self.inner.admission.admit(ticket, message)
    }

    /// Releases a coalescible pending event once it has been processed so a
    /// subsequent equivalent event is admitted rather than coalesced.
    pub fn release_coalescible(
        &self,
        device_key: cheetah_runtime_api::DeviceKey,
        class: TrafficClass,
    ) {
        self.inner.admission.release_coalescible(device_key, class);
    }

    /// Redrives up to `max` dead-lettered messages back onto their shard
    /// mailboxes once the runtime has recovered from overload. Returns the
    /// number of messages successfully re-admitted.
    pub fn redrive(&self, max: usize) -> Result<usize, RuntimeError> {
        self.inner.admission.redrive(max)
    }

    /// Returns `true` once [`Runtime::drain`] has begun.
    pub fn is_draining(&self) -> bool {
        self.inner.draining.load(Ordering::Acquire)
    }

    /// Gracefully drains the runtime: stops accepting new work, waits for
    /// in-flight shard backlog to be processed up to `deadline`, then shuts
    /// down the workers and timer wheel.
    ///
    /// Draining is idempotent: repeated calls simply observe an already-empty
    /// backlog and proceed to shutdown. After the first call, [`Runtime::send_message`]
    /// and [`Runtime::admit`] reject new work with [`RuntimeError::Draining`].
    pub async fn drain(&self, deadline: Duration) -> Result<DrainOutcome, RuntimeError> {
        self.inner.draining.store(true, Ordering::Release);

        let start = Instant::now();
        let poll = Duration::from_millis(DRAIN_POLL_INTERVAL_MS);
        let remaining = loop {
            let backlog: u64 = self.shard_mailbox_depths().iter().sum();
            if backlog == 0 {
                break 0;
            }
            if start.elapsed() >= deadline {
                break backlog;
            }
            tokio::time::sleep(poll).await;
        };

        self.shutdown().await?;
        Ok(DrainOutcome {
            drained_within_deadline: remaining == 0,
            remaining_backlog: remaining,
        })
    }

    /// Shuts down the runtime and waits for workers to finish.
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        let handles = self
            .inner
            .join_handles
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();

        for sender in self.inner.admission.senders().iter() {
            let sender = sender.clone();
            match tokio::time::timeout(
                Duration::from_millis(100),
                sender.send(RuntimeMessage::Shutdown),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {}
                Err(_) => {
                    return Err(RuntimeError::Internal(
                        "timed out sending shutdown to shard".into(),
                    ));
                }
            }
        }

        let timer_shutdown = self.inner.timer_shutdown_tx.clone();
        match tokio::time::timeout(Duration::from_millis(100), timer_shutdown.send(())).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {}
            Err(_) => {
                return Err(RuntimeError::Internal(
                    "timed out sending shutdown to timer wheel".into(),
                ));
            }
        }

        if let Some(handles) = handles {
            for handle in handles {
                match tokio::time::timeout(Duration::from_secs(5), handle).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        return Err(RuntimeError::Internal(e.to_string()));
                    }
                    Err(_) => {
                        return Err(RuntimeError::Internal(
                            "timed out waiting for worker to stop".into(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the runtime configuration.
    pub fn config(&self) -> &RuntimeConfig {
        &self.inner.config
    }

    /// Returns the session registry.
    pub fn session_registry(&self) -> &SessionRegistry<A::SessionHandle> {
        &self.inner.session_registry
    }

    /// Returns the admission controller.
    pub fn admission(&self) -> &dyn AdmissionControllerTrait {
        &self.inner.admission
    }

    /// Returns a point-in-time snapshot of runtime health metrics.
    pub fn metrics(&self) -> RuntimeMetricsSnapshot {
        self.inner.metrics.snapshot()
    }

    /// Returns the current occupancy of each shard mailbox, indexed by shard.
    pub fn shard_mailbox_depths(&self) -> Vec<u64> {
        self.inner.admission.shard_mailbox_depths()
    }

    /// Samples the current runtime state into `gb_metrics`.
    ///
    /// Feeds the runtime-derived GB28181 gauges (per-shard mailbox depth,
    /// active actors, timer lag) from a single consistent read. Intended to be
    /// called periodically by a supervising sampler.
    pub fn sample_gb_metrics(&self, gb_metrics: &crate::gb_metrics::GbMetrics) {
        gb_metrics.record_runtime_sample(&self.metrics(), &self.shard_mailbox_depths());
    }
}
