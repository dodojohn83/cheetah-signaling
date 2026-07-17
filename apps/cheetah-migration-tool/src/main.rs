//! CLI entry point for the Cheetah Signaling migration tool.

use cheetah_config::LayeredConfigSource;
use cheetah_migration_tool::clock::SystemClock;
use cheetah_migration_tool::error::MigrationError;
use cheetah_migration_tool::importer::{ImportOptions, Importer};
use cheetah_migration_tool::source::FileSource;
use cheetah_signal_types::ConfigSource;
use cheetah_signal_types::config::StorageBackend;
use cheetah_storage_api::Storage;
use cheetah_storage_postgres::PostgresStorage;
use cheetah_storage_sqlite::SqliteStorage;
use clap::Parser;
use secrecy::ExposeSecret;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "cheetah-migration-tool")]
#[command(about = "Import tenants, devices and channels into Cheetah Signaling")]
struct Args {
    /// Path to the Cheetah configuration file.
    #[arg(short, long)]
    config: PathBuf,
    /// Path to the source file (CSV or JSON array).
    #[arg(short, long)]
    source: PathBuf,
    /// Explicit source format. Inferred from the file extension when omitted.
    #[arg(short, long)]
    format: Option<String>,
    /// Dry-run: validate and summarize, but do not write to the target database.
    #[arg(long)]
    dry_run: bool,
    /// Optional file with a list of external IDs (one per line) to import for a cutover.
    #[arg(long)]
    cutover: Option<PathBuf>,
    /// Commit after this many records.
    #[arg(long, default_value = "100")]
    checkpoint_every: usize,
    /// Skip records that already exist in the target database.
    #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
    skip_existing: bool,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    if let Err(e) = run().await {
        tracing::error!(error = %e, "migration failed");
        std::process::exit(1);
    }
}

#[allow(clippy::print_stdout)]
async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    let config_source = LayeredConfigSource::new().with_config_path(&args.config);
    let signal_config = config_source.snapshot()?;
    let storage_config = signal_config.storage;

    let storage: Arc<dyn Storage> = match storage_config.backend {
        StorageBackend::Sqlite => {
            let storage = SqliteStorage::new(&storage_config.sqlite_path).await?;
            Arc::new(storage)
        }
        StorageBackend::Postgres => {
            let url = storage_config.postgres_url.expose_secret();
            let storage = PostgresStorage::new(url).await?;
            Arc::new(storage)
        }
        _ => {
            return Err(MigrationError::other(format!(
                "unsupported storage backend: {:?}",
                storage_config.backend
            )))?;
        }
    };

    storage.migration().run().await?;

    let source = FileSource::new(&args.source, args.format.map(|f| f.parse()).transpose()?)?;

    let cutover_ids = match args.cutover {
        Some(path) => parse_cutover_file(&path)?,
        None => std::collections::HashSet::new(),
    };

    if args.checkpoint_every == 0 {
        return Err(MigrationError::other(
            "--checkpoint-every must be at least 1",
        ))?;
    }

    let options = ImportOptions {
        checkpoint_every: args.checkpoint_every,
        dry_run: args.dry_run,
        skip_existing: args.skip_existing,
        cutover_ids,
    };

    let clock = Arc::new(SystemClock::new());
    let importer = Importer::new(storage, clock);
    let result = importer.import(&source, &options).await?;

    println!("Migration summary");
    println!("  records read:       {}", result.records_read);
    println!("  imported:           {}", result.records_imported);
    println!("  skipped (cutover):  {}", result.records_skipped);
    println!("  invalid:            {}", result.records_invalid);
    println!("  conflicting:        {}", result.records_conflicting);
    println!("  with secrets:       {}", result.records_with_secrets);

    if !result.counts_by_kind.is_empty() {
        println!("  by kind:");
        for (kind, count) in &result.counts_by_kind {
            println!("    {kind}: {count}");
        }
    }

    if !result.action_items.is_empty() {
        println!("Action items:");
        for item in &result.action_items {
            println!("  - {item}");
        }
    }

    if result.records_invalid > 0 {
        return Err("import completed with invalid records".into());
    }

    Ok(())
}

fn parse_cutover_file(
    path: &std::path::Path,
) -> Result<std::collections::HashSet<String>, Box<dyn std::error::Error + Send + Sync>> {
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(String::from)
        .collect())
}
