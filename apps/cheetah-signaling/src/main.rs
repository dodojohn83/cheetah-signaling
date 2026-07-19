//! Cheetah Signaling application binary.

mod assembly;
mod gb_event_sink;

use cheetah_signal_types::config::ConfigSource;
use tracing::info;

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
        gb28181 = ?runtime.gb28181_addr,
        "cheetah-signaling ready"
    );

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutdown signal received");
        }
    }

    runtime.shutdown();
    info!("cheetah-signaling stopped");
}
