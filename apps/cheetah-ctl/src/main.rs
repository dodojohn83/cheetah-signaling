//! Cheetah Signaling operational CLI.
//!
//! Wraps the administrative HTTP endpoints and local configuration validation
//! so operators can diagnose and recover a node without direct API calls.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use cheetah_signal_types::config::ConfigSource;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;

/// Operational CLI for Cheetah Signaling.
#[derive(Parser)]
#[command(name = "cheetah-ctl", about = "Cheetah Signaling operational CLI")]
struct Cli {
    /// Base URL of the signaling HTTP API.
    #[arg(
        long,
        short,
        env = "CHEETAH_BASE_URL",
        default_value = "http://localhost:8080"
    )]
    base_url: String,

    /// API key with `system_admin` scope.
    #[arg(long, short, env = "CHEETAH_API_KEY")]
    api_key: Option<String>,

    /// Tenant identifier. Required for `device-diagnostics` and sent as `x-tenant-id`.
    #[arg(long, short, env = "CHEETAH_TENANT_ID")]
    tenant: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a local configuration file without applying it.
    ValidateConfig {
        /// Path to a TOML configuration file.
        #[arg(value_name = "CONFIG")]
        config: PathBuf,
    },
    /// Show database migration status.
    DbStatus,
    /// Run pending database migrations.
    DbMigrate,
    /// Request a graceful node drain.
    NodeDrain,
    /// Replay the outbox from the earliest unprocessed entry.
    OutboxReplay,
    /// Trigger a reconciliation pass.
    Reconcile,
    /// Request a sanitized diagnostics package for a device.
    DeviceDiagnostics {
        /// Device identifier.
        id: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    match cli.command {
        Command::ValidateConfig { config } => {
            validate_config(config);
            Ok(())
        }
        Command::DbStatus => admin_get(&cli, "/api/v1/admin/db-status").await,
        Command::DbMigrate => admin_post(&cli, "/api/v1/admin/db-migrate").await,
        Command::NodeDrain => admin_post(&cli, "/api/v1/admin/node-drain").await,
        Command::OutboxReplay => admin_post(&cli, "/api/v1/admin/outbox-replay").await,
        Command::Reconcile => admin_post(&cli, "/api/v1/admin/reconcile").await,
        Command::DeviceDiagnostics { ref id } => {
            if cli.tenant.is_none() {
                eprintln!(
                    "{}",
                    serde_json::json!({"error": "--tenant is required for device-diagnostics"})
                );
                std::process::exit(2);
            }
            let url = device_diagnostics_url(&cli.base_url, id)?;
            admin_get_url(&cli, url).await
        }
    }
}

fn validate_config(path: PathBuf) {
    let source = cheetah_config::LayeredConfigSource::new().with_config_path(path);
    match source.snapshot() {
        Ok(_) => println!("{}", serde_json::json!({"valid": true})),
        Err(e) => {
            eprintln!(
                "{}",
                serde_json::json!({"valid": false, "error": e.to_string()})
            );
            std::process::exit(2);
        }
    }
}

/// Builds an admin endpoint URL by appending `path` to `base_url`.
fn admin_url(
    base: &str,
    path: &str,
) -> Result<reqwest::Url, Box<dyn std::error::Error + Send + Sync>> {
    let mut url = reqwest::Url::parse(base)?;
    url.path_segments_mut()
        .map_err(|_| "base URL cannot be used as a base for path segments")?
        .pop_if_empty()
        .extend(path.split('/').filter(|s| !s.is_empty()));
    Ok(url)
}

/// Builds a device diagnostics URL, percent-encoding the device identifier.
fn device_diagnostics_url(
    base: &str,
    id: &str,
) -> Result<reqwest::Url, Box<dyn std::error::Error + Send + Sync>> {
    if id.is_empty() {
        return Err("device id must not be empty".into());
    }
    let mut url = reqwest::Url::parse(base)?;
    {
        let mut segments = url
            .path_segments_mut()
            .map_err(|_| "base URL cannot be used as a base for path segments")?;
        segments.pop_if_empty();
        segments.extend(["api", "v1", "admin", "devices"]);
        segments.push(id);
        segments.push("diagnostics");
    }
    Ok(url)
}

fn build_client(cli: &Cli) -> Result<reqwest::Client, Box<dyn std::error::Error + Send + Sync>> {
    Ok(reqwest::Client::builder()
        .default_headers(build_headers(cli)?)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()?)
}

fn build_headers(
    cli: &Cli,
) -> Result<reqwest::header::HeaderMap, Box<dyn std::error::Error + Send + Sync>> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("accept"),
        HeaderValue::from_static("application/json"),
    );
    if let Some(key) = &cli.api_key {
        let value = HeaderValue::from_str(key)?;
        headers.insert("X-Api-Key", value);
    }
    if let Some(tenant) = &cli.tenant {
        let value = HeaderValue::from_str(tenant)?;
        headers.insert("x-tenant-id", value);
    }
    Ok(headers)
}

async fn admin_get(cli: &Cli, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = admin_url(&cli.base_url, path)?;
    admin_get_url(cli, url).await
}

async fn admin_post(cli: &Cli, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = admin_url(&cli.base_url, path)?;
    admin_post_url(cli, url).await
}

async fn admin_get_url(
    cli: &Cli,
    url: reqwest::Url,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_client(cli)?;
    let response = client.get(url).send().await?;
    handle_response(response).await
}

async fn admin_post_url(
    cli: &Cli,
    url: reqwest::Url,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_client(cli)?;
    let response = client.post(url).send().await?;
    handle_response(response).await
}

async fn handle_response(
    response: reqwest::Response,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        eprintln!(
            "{}",
            serde_json::json!({"status": status.as_u16(), "body": body})
        );
        std::process::exit(1);
    }

    if body.trim().is_empty() {
        println!("{{}}");
    } else {
        println!("{body}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_url_appends_path_to_base() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = admin_url("http://localhost:8080", "/api/v1/admin/db-status")?;
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/admin/db-status");
        Ok(())
    }

    #[test]
    fn admin_url_works_with_trailing_slash() -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    {
        let url = admin_url("http://localhost:8080/", "/api/v1/admin/db-status")?;
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/admin/db-status");
        Ok(())
    }

    #[test]
    fn admin_url_strips_empty_trailing_path_segment()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = admin_url("https://gw.example.com/cheetah/", "/api/v1/admin/db-status")?;
        assert_eq!(
            url.as_str(),
            "https://gw.example.com/cheetah/api/v1/admin/db-status"
        );
        Ok(())
    }

    #[test]
    fn device_diagnostics_url_encodes_id() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = device_diagnostics_url("http://localhost:8080", "dev/ 1")?;
        assert_eq!(
            url.as_str(),
            "http://localhost:8080/api/v1/admin/devices/dev%2F%201/diagnostics"
        );
        Ok(())
    }

    #[test]
    fn device_diagnostics_url_strips_empty_trailing_path_segment()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = device_diagnostics_url("https://gw.example.com/cheetah/", "dev-1")?;
        assert_eq!(
            url.as_str(),
            "https://gw.example.com/cheetah/api/v1/admin/devices/dev-1/diagnostics"
        );
        Ok(())
    }

    #[test]
    fn device_diagnostics_url_rejects_empty_id() {
        assert!(device_diagnostics_url("http://localhost:8080", "").is_err());
    }

    #[test]
    fn build_headers_rejects_invalid_header_values() {
        let cli = Cli {
            base_url: "http://localhost:8080".to_string(),
            api_key: Some("bad\nvalue".to_string()),
            tenant: None,
            command: Command::DbStatus,
        };
        assert!(build_headers(&cli).is_err());
    }

    #[test]
    fn build_headers_sets_accept_api_key_and_tenant()
    -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

        let cli = Cli {
            base_url: "http://localhost:8080".to_string(),
            api_key: Some("secret-key".to_string()),
            tenant: Some("tenant-42".to_string()),
            command: Command::DbStatus,
        };
        let headers = build_headers(&cli)?;
        let mut expected = HeaderMap::new();
        expected.insert(
            HeaderName::from_static("accept"),
            HeaderValue::from_static("application/json"),
        );
        expected.insert("X-Api-Key", HeaderValue::from_static("secret-key"));
        expected.insert("x-tenant-id", HeaderValue::from_static("tenant-42"));
        assert_eq!(headers, expected);
        Ok(())
    }
}
