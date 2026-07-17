//! Cheetah Signaling application binary.

use cheetah_config::LayeredConfigSource;
use cheetah_signal_types::ConfigSource;

#[allow(clippy::print_stderr)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = LayeredConfigSource::new();
    let config = source.snapshot()?;

    // Sanitized startup summary: selected non-secret configuration fields only.
    eprintln!("Cheetah Signaling starting");
    eprintln!("  node: {}", config.system.node_name);
    eprintln!("  data dir: {}", config.system.data_dir);
    eprintln!("  http: {}:{}", config.http.listen_addr, config.http.port);
    eprintln!("  grpc: {}:{}", config.grpc.listen_addr, config.grpc.port);
    eprintln!("  storage backend: {:?}", config.storage.backend);
    eprintln!("  messaging backend: {:?}", config.messaging.backend);
    eprintln!("  log level: {}", config.system.log_level);

    // Full startup sequence (runtime, bus, repositories, protocols) is
    // implemented in later phases of 002_vibe_coding_plan.

    Ok(())
}
