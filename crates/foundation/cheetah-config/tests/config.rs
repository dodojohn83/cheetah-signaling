//! Integration tests for cheetah-config.

use cheetah_config::LayeredConfigSource;
use cheetah_signal_types::{ConfigSource, DurationMs, SignalError};
use std::fs;
use std::path::PathBuf;

fn temp_config_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "cheetah-config-test-{}-{}.toml",
        name,
        std::process::id()
    ));
    path
}

#[test]
fn default_config_loads_and_validates() -> Result<(), SignalError> {
    let source = LayeredConfigSource::new();
    let config = source.snapshot()?;
    assert_eq!(config.http.port, 8080);
    assert_eq!(config.grpc.port, 50051);
    assert_eq!(config.runtime.worker_threads, 4);
    Ok(())
}

#[test]
fn file_override_changes_http_port() -> Result<(), SignalError> {
    let path = temp_config_path("http-port");
    let content = r#"
[http]
port = 9090
"#;
    fs::write(&path, content).map_err(|e| {
        SignalError::new(
            cheetah_signal_types::SignalErrorKind::Internal,
            "failed to write temp file",
        )
        .with_source(e)
    })?;

    let source = LayeredConfigSource::new().with_config_path(&path);
    let config = source.snapshot()?;
    assert_eq!(config.http.port, 9090);

    let _ = fs::remove_file(&path);
    Ok(())
}

#[test]
fn file_override_round_trips_duration_ms() -> Result<(), SignalError> {
    let path = temp_config_path("duration");
    let content = r#"
[media]
default_invite_timeout_ms = 60000
"#;
    fs::write(&path, content).map_err(|e| {
        SignalError::new(
            cheetah_signal_types::SignalErrorKind::Internal,
            "failed to write temp file",
        )
        .with_source(e)
    })?;

    let source = LayeredConfigSource::new().with_config_path(&path);
    let config = source.snapshot()?;
    assert_eq!(
        config.media.default_invite_timeout_ms,
        DurationMs::from_seconds(60)
    );

    let _ = fs::remove_file(&path);
    Ok(())
}

#[test]
fn invalid_config_fails_validation() {
    let path = temp_config_path("invalid");
    let content = r#"
[runtime]
worker_threads = 0
"#;
    let _ = fs::write(&path, content);

    let source = LayeredConfigSource::new().with_config_path(&path);
    let result = source.snapshot();
    assert!(result.is_err());

    let _ = fs::remove_file(&path);
}
