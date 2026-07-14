//! Default `Clock` implementation for the Tokio runtime.

use std::time::Instant;

use cheetah_signal_types::{Clock, DurationMs, UtcTimestamp};

/// A clock backed by the operating system.
#[derive(Debug)]
pub(crate) struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Creates a new system clock.
    pub(crate) fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now_wall(&self) -> UtcTimestamp {
        UtcTimestamp::from_offset(time::OffsetDateTime::now_utc())
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::from_millis(self.start.elapsed().as_millis() as i64)
    }
}
