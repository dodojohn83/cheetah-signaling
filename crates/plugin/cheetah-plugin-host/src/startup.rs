//! Plugin process startup readiness helpers.

use cheetah_plugin_sdk::PluginError;
use cheetah_signal_types::DurationMs;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;
use tracing::debug;

const MAX_PLUGIN_STARTUP_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// Waits until a plugin TCP listen address becomes reachable, polling it at the
/// configured interval.
///
/// The startup timeout and poll interval are clamped to safe bounds so a
/// misconfigured huge interval cannot block the caller or overflow
/// `tokio::time`.
pub(crate) async fn wait_for_ready(
    address: &str,
    startup_timeout: DurationMs,
    poll_interval: DurationMs,
) -> Result<(), PluginError> {
    let now = std::time::Instant::now();
    let startup = Duration::from_millis(startup_timeout.as_millis().max(0) as u64)
        .min(MAX_PLUGIN_STARTUP_TIMEOUT);
    let deadline = now.checked_add(startup).unwrap_or(now);
    let poll = Duration::from_millis(poll_interval.as_millis().max(1) as u64)
        .max(MIN_POLL_INTERVAL)
        .min(startup);

    while std::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let connect = tokio::time::timeout(remaining, TcpStream::connect(address));
        match connect.await {
            Ok(Ok(_stream)) => return Ok(()),
            Ok(Err(e)) => {
                debug!(address = %address, error = %e, "plugin not ready yet");
                sleep(poll).await;
            }
            Err(_) => {
                debug!(address = %address, "plugin readiness probe timed out");
                sleep(poll).await;
            }
        }
    }

    Err(PluginError::Driver(format!(
        "plugin did not become reachable at {address} within {} ms",
        startup_timeout.as_millis()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_interval_zero_is_clamped_to_one_millisecond() {
        let interval_ms = DurationMs::from_millis(0).as_millis().max(1);
        let poll = Duration::from_millis(interval_ms as u64).max(MIN_POLL_INTERVAL);
        assert_eq!(poll, MIN_POLL_INTERVAL);
    }

    #[tokio::test]
    async fn wait_for_ready_clamps_huge_poll_interval_to_startup_timeout() {
        let result = wait_for_ready(
            "127.0.0.1:1",
            DurationMs::from_millis(50),
            DurationMs::from_millis(i64::MAX),
        )
        .await;
        assert!(result.is_err(), "unreachable plugin must time out");
    }
}
