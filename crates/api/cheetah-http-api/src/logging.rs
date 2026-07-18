//! Tracing subscriber setup and protocol-body logging controls.

use cheetah_signal_types::config::LogFormat;
use std::sync::OnceLock;
use tracing_subscriber::EnvFilter;

static TRACING_INIT: OnceLock<()> = OnceLock::new();

/// Initializes the process-wide tracing subscriber once.
///
/// Defaults to JSON output at the configured level. In edge interactive mode
/// `LogFormat::Compact` uses a compact human-readable formatter. The global
/// subscriber is only set on the first call; subsequent calls are ignored so
/// tests and application restarts do not panic.
pub fn init_tracing(log_level: &str, log_format: LogFormat) {
    TRACING_INIT.get_or_init(|| {
        let level = if log_level.trim().is_empty() {
            "info"
        } else {
            log_level
        };
        let filter = match EnvFilter::try_new(level) {
            Ok(filter) => filter,
            Err(_) => EnvFilter::new("info"),
        };

        match log_format {
            LogFormat::Json => {
                let _ = tracing_subscriber::fmt()
                    .json()
                    .with_env_filter(filter)
                    .try_init();
            }
            LogFormat::Compact => {
                let _ = tracing_subscriber::fmt()
                    .compact()
                    .with_env_filter(filter)
                    .try_init();
            }
        }
    });
}
