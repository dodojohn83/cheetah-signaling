//! Cheetah Signaling application binary.

use cheetah_signal_types::config::ConfigSource;

#[allow(clippy::print_stderr)]
fn main() {
    // Load configuration so that observability can be initialized before
    // the full transport/adapters are assembled. The complete startup
    // sequence (storage, bus, ownership, media, protocol drivers, HTTP/gRPC)
    // is implemented in later phases.
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
}
