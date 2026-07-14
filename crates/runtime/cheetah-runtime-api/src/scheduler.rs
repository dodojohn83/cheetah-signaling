//! Timer scheduler port.

use async_trait::async_trait;

use cheetah_signal_types::DurationMs;

use crate::{DeviceKey, RuntimeError, TimerId};

/// Schedules and cancels per-device timers.
#[async_trait]
pub trait Scheduler: Send + Sync + 'static {
    /// Schedules a timer to fire after the given delay.
    async fn schedule(
        &self,
        device_key: DeviceKey,
        timer_id: TimerId,
        delay: DurationMs,
        kind: String,
    ) -> Result<(), RuntimeError>;

    /// Cancels a previously scheduled timer.
    async fn cancel(&self, device_key: DeviceKey, timer_id: TimerId) -> Result<(), RuntimeError>;
}
