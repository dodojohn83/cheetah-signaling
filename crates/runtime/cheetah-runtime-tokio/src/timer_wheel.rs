//! Hierarchical timer wheel for heartbeat, registration, and operation deadlines.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cheetah_runtime_api::{DeviceKey, RuntimeMessage, RuntimeMetrics, ShardRouter, TimerId};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};

/// Command sent to the timer wheel.
#[derive(Clone, Debug)]
pub(crate) enum TimerCommand {
    /// Schedule a new timer.
    Schedule {
        /// Device that owns the timer.
        device_key: DeviceKey,
        /// Timer identifier.
        timer_id: TimerId,
        /// Absolute monotonic deadline.
        deadline: Instant,
        /// Caller-provided timer kind.
        kind: String,
    },
    /// Cancel a previously scheduled timer.
    Cancel {
        /// Device that owns the timer.
        device_key: DeviceKey,
        /// Timer identifier.
        timer_id: TimerId,
    },
}

/// Internal timer entry.
#[derive(Clone, Debug)]
struct TimerEntry {
    device_key: DeviceKey,
    timer_id: TimerId,
    kind: String,
}

/// Hierarchical timer wheel.
pub(crate) struct TimerWheel;

impl TimerWheel {
    /// Starts the timer wheel task.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run(
        mut commands_rx: mpsc::Receiver<TimerCommand>,
        mut shutdown_rx: mpsc::Receiver<()>,
        senders: Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
        router: ShardRouter,
        tick_resolution_ms: u64,
        max_pending_dispatch: usize,
        metrics: Arc<RuntimeMetrics>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(async move {
            let mut interval = interval(Duration::from_millis(tick_resolution_ms));
            let mut pending_dispatch: VecDeque<RuntimeMessage> = VecDeque::new();
            let mut timers_by_deadline: BTreeMap<Instant, Vec<TimerEntry>> = BTreeMap::new();
            let mut timer_map: BTreeMap<TimerId, Instant> = BTreeMap::new();

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let now = Instant::now();
                        process_expired(
                            &senders,
                            &router,
                            &mut timers_by_deadline,
                            &mut timer_map,
                            &mut pending_dispatch,
                            now,
                            max_pending_dispatch,
                            &metrics,
                        );
                        process_pending_dispatch(
                            &senders,
                            &router,
                            &mut pending_dispatch,
                            max_pending_dispatch,
                            &metrics,
                        );
                        metrics.set_pending_timer_dispatch(pending_dispatch.len() as u64);
                    }
                    cmd = commands_rx.recv() => {
                        match cmd {
                            Some(cmd) => process_command(
                                cmd,
                                &mut timers_by_deadline,
                                &mut timer_map,
                                &mut pending_dispatch,
                                max_pending_dispatch,
                                &metrics,
                            ),
                            None => break,
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        })
    }
}

fn interval(period: Duration) -> Interval {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

fn process_command(
    cmd: TimerCommand,
    timers_by_deadline: &mut BTreeMap<Instant, Vec<TimerEntry>>,
    timer_map: &mut BTreeMap<TimerId, Instant>,
    pending_dispatch: &mut VecDeque<RuntimeMessage>,
    max_pending_dispatch: usize,
    metrics: &RuntimeMetrics,
) {
    match cmd {
        TimerCommand::Schedule {
            device_key,
            timer_id,
            deadline,
            kind,
        } => {
            timer_map.insert(timer_id, deadline);
            timers_by_deadline
                .entry(deadline)
                .or_default()
                .push(TimerEntry {
                    device_key,
                    timer_id,
                    kind,
                });
            metrics.record_timer_scheduled();
        }
        TimerCommand::Cancel {
            timer_id,
            device_key: _device_key,
        } => {
            if let Some(deadline) = timer_map.remove(&timer_id)
                && let Some(timers) = timers_by_deadline.get_mut(&deadline)
            {
                timers.retain(|entry| entry.timer_id != timer_id);
                if timers.is_empty() {
                    timers_by_deadline.remove(&deadline);
                }
            }
            pending_dispatch.retain(
                |msg| !matches!(msg, RuntimeMessage::Timer { timer_id: id, .. } if *id == timer_id),
            );
            enforce_limit(pending_dispatch, max_pending_dispatch, metrics);
            metrics.record_timer_cancelled();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn process_expired(
    senders: &Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
    router: &ShardRouter,
    timers_by_deadline: &mut BTreeMap<Instant, Vec<TimerEntry>>,
    timer_map: &mut BTreeMap<TimerId, Instant>,
    pending_dispatch: &mut VecDeque<RuntimeMessage>,
    now: Instant,
    max_pending_dispatch: usize,
    metrics: &RuntimeMetrics,
) {
    while let Some((deadline, timers)) = timers_by_deadline.pop_first() {
        if deadline > now {
            timers_by_deadline.insert(deadline, timers);
            break;
        }

        for entry in timers {
            if timer_map.remove(&entry.timer_id).is_some() {
                metrics.record_timer_fired();
                let msg = RuntimeMessage::Timer {
                    device_key: entry.device_key,
                    timer_id: entry.timer_id,
                    kind: entry.kind,
                };
                if let Err(msg) = dispatch(senders, router, msg) {
                    pending_dispatch.push_back(msg);
                    enforce_limit(pending_dispatch, max_pending_dispatch, metrics);
                }
            }
        }
    }
}

fn process_pending_dispatch(
    senders: &Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
    router: &ShardRouter,
    pending_dispatch: &mut VecDeque<RuntimeMessage>,
    max_pending_dispatch: usize,
    metrics: &RuntimeMetrics,
) {
    let mut remaining = VecDeque::new();
    let pending = std::mem::take(pending_dispatch);

    for msg in pending {
        if let Err(msg) = dispatch(senders, router, msg) {
            remaining.push_back(msg);
        }
    }

    *pending_dispatch = remaining;
    enforce_limit(pending_dispatch, max_pending_dispatch, metrics);
}

fn dispatch(
    senders: &Arc<Vec<mpsc::Sender<RuntimeMessage>>>,
    router: &ShardRouter,
    msg: RuntimeMessage,
) -> Result<(), RuntimeMessage> {
    let device_key = match msg.device_key() {
        Some(key) => key,
        None => return Err(msg),
    };
    let index = router.route(device_key);
    let sender = match senders.get(index) {
        Some(s) => s,
        None => return Err(msg),
    };
    match sender.try_send(msg) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(msg)) => Err(msg),
        Err(mpsc::error::TrySendError::Closed(_msg)) => {
            tracing::warn!("shard {index} closed; dropping timer");
            Ok(())
        }
    }
}

fn enforce_limit(
    pending_dispatch: &mut VecDeque<RuntimeMessage>,
    max_pending_dispatch: usize,
    metrics: &RuntimeMetrics,
) {
    let overflow = pending_dispatch.len().saturating_sub(max_pending_dispatch);
    if overflow > 0 {
        tracing::warn!(
            dropped = overflow,
            max_pending_dispatch,
            "timer dispatch queue overflow; dropping oldest timers"
        );
        metrics.record_timers_dropped(overflow as u64);
    }
    while pending_dispatch.len() > max_pending_dispatch {
        pending_dispatch.pop_front();
    }
}
