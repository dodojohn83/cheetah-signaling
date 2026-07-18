//! Wall-clock and monotonic time source for the migration tool.

use cheetah_signal_types::{Clock, DurationMs, UtcTimestamp};
use std::time::{Instant, SystemTime};
use time::{OffsetDateTime, UtcOffset};

/// System clock used outside of tests.
#[derive(Debug, Clone)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    /// Creates a new system clock.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_wall(&self) -> UtcTimestamp {
        let dt = OffsetDateTime::from(SystemTime::now());
        UtcTimestamp::from_offset(dt.to_offset(UtcOffset::UTC))
    }

    fn now_monotonic(&self) -> DurationMs {
        DurationMs::from_millis(Instant::now().duration_since(self.start).as_millis() as i64)
    }
}
