//! Small, stateless helpers shared by the ONVIF driver modules.

use std::time::{Duration, Instant};

/// Upper bound for any ONVIF timeout (one day). Prevents `Instant` overflow
/// and `tokio::time` deadline panics when a caller supplies an extremely large
/// duration.
pub(crate) const MAX_ONVIF_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// Clamps `timeout` to [`MAX_ONVIF_TIMEOUT`].
pub(crate) fn clamp_timeout(timeout: Duration) -> Duration {
    timeout.min(MAX_ONVIF_TIMEOUT)
}

/// Computes a deadline `Instant` from an optional timeout, capping the result
/// so it cannot overflow the platform `Instant` range.
pub(crate) fn deadline_from_now(timeout: Option<Duration>) -> Option<Instant> {
    timeout.map(|d| {
        let now = Instant::now();
        now.checked_add(d.min(MAX_ONVIF_TIMEOUT))
            .or_else(|| now.checked_add(MAX_ONVIF_TIMEOUT))
            .unwrap_or(now)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn clamp_timeout_saturates_at_one_day() {
        assert_eq!(clamp_timeout(MAX_ONVIF_TIMEOUT), MAX_ONVIF_TIMEOUT);
        assert_eq!(
            clamp_timeout(Duration::from_millis(u64::MAX)),
            MAX_ONVIF_TIMEOUT
        );
    }

    #[test]
    fn deadline_from_now_does_not_panic_with_huge_timeout() {
        let now = Instant::now();
        let deadline = deadline_from_now(Some(Duration::from_millis(u64::MAX))).unwrap();
        assert!(deadline >= now, "deadline must not be in the past");
    }
}
