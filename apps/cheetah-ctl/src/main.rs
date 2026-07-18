//! Cheetah Signaling operational CLI.
//!
//! Wraps the administrative HTTP endpoints and local configuration validation
//! so operators can diagnose and recover a node without direct API calls.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use cheetah_signal_types::config::ConfigSource;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
            let path = format!("/api/v1/admin/devices/{id}/diagnostics");
            admin_get(&cli, &path).await
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

fn admin_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    format!("{base}{path}")
}

fn build_client(cli: &Cli) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(key) = &cli.api_key
        && let Ok(value) = reqwest::header::HeaderValue::from_str(key)
    {
        headers.insert("X-Api-Key", value);
    }
    if let Some(tenant) = &cli.tenant
        && let Ok(value) = reqwest::header::HeaderValue::from_str(tenant)
    {
        headers.insert("x-tenant-id", value);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

async fn admin_get(cli: &Cli, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_client(cli);
    let url = admin_url(&cli.base_url, path);
    let response = client.get(&url).send().await?;
    handle_response(response).await
}

async fn admin_post(cli: &Cli, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = build_client(cli);
    let url = admin_url(&cli.base_url, path);
    let response = client.post(&url).send().await?;
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
