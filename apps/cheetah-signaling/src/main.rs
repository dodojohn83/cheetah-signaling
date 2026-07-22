//! Cheetah Signaling application binary.

mod assembly;
mod gb_catalog_buffer;
mod gb_event_sink;
mod onvif_discovery;
mod operation_dispatch_worker;
mod periodic_reconcile_worker;
mod workers;

use cheetah_signal_types::config::ConfigSource;
use std::time::Duration;
use tracing::{info, warn};

#[tokio::main]
#[allow(clippy::print_stderr)]
async fn main() {
    let config_source = cheetah_config::LayeredConfigSource::new();
    let config = match config_source.snapshot() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("failed to load configuration: {e:#}");
            std::process::exit(1);
        }
    };

    cheetah_http_api::logging::init_tracing(
        &config.system.log_level,
        config.observability.log_format,
    );

    let runtime = match Box::pin(assembly::start(config)).await {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("failed to start signaling process: {e:#}");
            std::process::exit(1);
        }
    };

    info!(
        http = %runtime.http_addr,
        grpc = %runtime.grpc_addr,
        gb28181 = ?runtime.gb28181_addr,
        ready = runtime.ready.load(std::sync::atomic::Ordering::SeqCst),
        "cheetah-signaling running"
    );

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
        }
    }

    let health = runtime.shutdown(Duration::from_secs(30)).await;
    if health
        .components
        .values()
        .any(|s| matches!(s, assembly::ComponentStatus::Failed(_)))
    {
        warn!(?health, "shutdown completed with worker failures");
    } else {
        info!(?health, "cheetah-signaling stopped");
    }
}
