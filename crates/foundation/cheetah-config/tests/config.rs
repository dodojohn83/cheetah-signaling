//! Integration tests for cheetah-config.

use cheetah_config::LayeredConfigSource;
use cheetah_signal_types::{ConfigSource, DeploymentProfile, DurationMs, SignalError};
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

#[test]
fn cluster_profile_requires_postgres_nats_and_cluster_enabled() -> Result<(), SignalError> {
    let path = temp_config_path("cluster");
    let content = r#"
[system]
profile = "cluster"

[storage]
backend = "postgres"
postgres_url = "postgres://u:p@localhost/cheetah"

[messaging]
backend = "nats"

[cluster]
enabled = true

[grpc]
tls_cert_ref = "certs/grpc.crt"
tls_key_ref = "certs/grpc.key"
mtls_client_ca_ref = "certs/grpc-client-ca.crt"
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
    assert_eq!(config.system.profile, Some(DeploymentProfile::Cluster));

    let _ = fs::remove_file(&path);
    Ok(())
}

#[test]
fn cluster_profile_is_inferred_when_omitted() -> Result<(), SignalError> {
    let path = temp_config_path("cluster-inferred");
    let content = r#"
[storage]
backend = "postgres"
postgres_url = "postgres://u:p@localhost/cheetah"

[messaging]
backend = "nats"

[cluster]
enabled = true

[grpc]
tls_cert_ref = "certs/grpc.crt"
tls_key_ref = "certs/grpc.key"
mtls_client_ca_ref = "certs/grpc-client-ca.crt"
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
    assert_eq!(config.system.profile, None);

    let _ = fs::remove_file(&path);
    Ok(())
}

#[test]
fn edge_profile_rejects_postgres_backend() {
    let path = temp_config_path("edge-postgres");
    let content = r#"
[system]
profile = "edge"

[storage]
backend = "postgres"
"#;
    let _ = fs::write(&path, content);

    let source = LayeredConfigSource::new().with_config_path(&path);
    let result = source.snapshot();
    assert!(result.is_err());

    let _ = fs::remove_file(&path);
}
