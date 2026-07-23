//! CLI entry point for the deterministic GB28181 simulator.
//!
//! The simulator runs a reproducible, discrete-event scenario and prints a JSON
//! [`RunReport`].  A scenario is either loaded from a TOML file (`--scenario`)
//! or synthesized from a small set of flags for quick smoke runs.

use cheetah_gb28181_simulator::scenario::{
    Direction, FaultKind, MessageClass, Profile, Scenario, ScenarioError, Transport,
};
use cheetah_gb28181_simulator::{RunReport, run_scenario};
use clap::Parser;
use std::path::PathBuf;

/// CLI arguments.
#[derive(Debug, Parser)]
#[command(name = "gb28181-simulator")]
#[command(about = "Deterministic fixed-shard GB28181 signalling simulator")]
struct Args {
    /// Scenario TOML file.  When omitted, a scenario is built from flags.
    #[arg(long)]
    scenario: Option<PathBuf>,

    /// Master seed (ignored when --scenario provides one).
    #[arg(long, default_value = "0")]
    seed: u64,

    /// Number of shard workers (flag mode).
    #[arg(long, default_value = "4")]
    shards: u32,

    /// Number of devices (flag mode).
    #[arg(long, default_value = "10")]
    count: u32,

    /// Transport (flag mode): udp or tcp.
    #[arg(long, default_value = "udp")]
    transport: String,

    /// Virtual run duration in milliseconds (flag mode).
    #[arg(long, default_value = "120000")]
    duration_ms: u64,

    /// Profile id (flag mode).
    #[arg(long, default_value = "generic")]
    profile: String,

    /// Uniform drop rate applied to device->platform frames (flag mode).
    #[arg(long, default_value = "0.0")]
    drop_rate: f64,

    /// Write the report JSON to this file instead of stdout.
    #[arg(long)]
    report: Option<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let scenario = build_scenario(&args)?;
    let report = run_scenario(scenario);
    emit_report(&report, args.report.as_deref())?;
    Ok(())
}

fn build_scenario(args: &Args) -> Result<Scenario, ScenarioError> {
    if let Some(path) = &args.scenario {
        return Scenario::from_toml_path(path);
    }
    let transport = if args.transport.eq_ignore_ascii_case("tcp") {
        Transport::Tcp
    } else {
        Transport::Udp
    };
    let mut faults = Vec::new();
    if args.drop_rate > 0.0 {
        faults.push(FaultKind::Drop {
            rate: args.drop_rate,
            direction: Direction::DeviceToPlatform,
            target: MessageClass::Any,
        });
    }
    let scenario = Scenario {
        name: "cli".to_string(),
        seed: args.seed,
        shards: args.shards,
        device_count: args.count,
        transport,
        duration_ms: args.duration_ms,
        profile: Profile {
            id: args.profile.clone(),
            ..Profile::default()
        },
        faults,
        ..Scenario::default()
    };
    scenario.validate()?;
    Ok(scenario)
}

fn emit_report(
    report: &RunReport,
    path: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = report.to_json()?;
    match path {
        Some(path) => std::fs::write(path, json)?,
        None => {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(json.as_bytes())?;
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}
