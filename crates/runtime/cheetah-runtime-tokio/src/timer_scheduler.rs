//! Tokio implementation of the `Scheduler` port.

use async_trait::async_trait;

use cheetah_runtime_api::{DeviceKey, RuntimeError, Scheduler, TimerId};
use cheetah_signal_types::DurationMs;
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::timer_wheel::TimerCommand;

const FAR_FUTURE: std::time::Duration = std::time::Duration::from_secs(60 * 60 * 24 * 365 * 10);

/// Sends schedule/cancel commands to the timer wheel.
#[derive(Clone)]
pub(crate) struct TimerScheduler {
    timer_tx: mpsc::Sender<TimerCommand>,
}

impl std::fmt::Debug for TimerScheduler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimerScheduler").finish_non_exhaustive()
    }
}

impl TimerScheduler {
    /// Creates a new timer scheduler with the given command channel.
    pub(crate) fn new(timer_tx: mpsc::Sender<TimerCommand>) -> Self {
        Self { timer_tx }
    }
}

#[async_trait]
impl Scheduler for TimerScheduler {
    async fn schedule(
        &self,
        device_key: DeviceKey,
        timer_id: TimerId,
        delay: DurationMs,
        kind: String,
    ) -> Result<(), RuntimeError> {
        let ms = delay.as_millis().max(0) as u64;
        let duration = std::time::Duration::from_millis(ms);
        let now = Instant::now();
        let far = now.checked_add(FAR_FUTURE).unwrap_or(now);
        let deadline = now.checked_add(duration).unwrap_or(far);
        self.timer_tx
            .send(TimerCommand::Schedule {
                device_key,
                timer_id,
                deadline,
                kind,
            })
            .await
            .map_err(|_| RuntimeError::Shutdown)
    }

    async fn cancel(&self, device_key: DeviceKey, timer_id: TimerId) -> Result<(), RuntimeError> {
        self.timer_tx
            .send(TimerCommand::Cancel {
                device_key,
                timer_id,
            })
            .await
            .map_err(|_| RuntimeError::Shutdown)
    }
}
