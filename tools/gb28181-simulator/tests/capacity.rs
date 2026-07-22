//! GB4-SYS-007: bounded synthetic capacity harness.
//!
//! Runs the development-scale `capacity-dev.toml` profile (the bounded stand-in
//! for the 100k / 300k / 1M release profiles documented in
//! `dev-docs/004_gb28181-improve/reports/gb4-sys-007.md`) and validates the
//! capacity invariants the simulator can measure deterministically:
//!
//! - every synthetic device converges (register + keepalive + catalog + media
//!   control), yielding register/keepalive/operation throughput;
//! - resources stay bounded: fixed shard/socket/pool counts, and the peak
//!   scheduled-event backlog scales with the device population rather than
//!   growing without bound (no per-device task/timer explosion);
//! - the run is reproducible (identical transcript hash).
//!
//! Marked `#[ignore]` so it runs only on demand:
//! `cargo test -p cheetah-gb28181-simulator --test capacity -- --ignored`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use cheetah_gb28181_simulator::{RunReport, Scenario, run_scenario};

fn load_capacity() -> Scenario {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios/capacity-dev.toml");
    Scenario::from_toml_path(path).expect("capacity-dev.toml parses")
}

fn print_capacity_metrics(report: &RunReport) {
    let duration_s = report.duration_ms as f64 / 1000.0;
    let register_tps = report.outcomes.registrations_accepted as f64 / duration_s;
    let keepalive_tps = report.outcomes.keepalives_acked as f64 / duration_s;
    let message_tps = report.message_counts.sent as f64 / duration_s;
    eprintln!(
        "CAPACITY {{\n  \"scenario\": \"{}\",\n  \"devices\": {},\n  \"virtual_duration_s\": {:.1},\n  \"register_tps\": {:.2},\n  \"keepalive_tps\": {:.2},\n  \"message_tps\": {:.2},\n  \"shards\": {},\n  \"udp_sockets\": {},\n  \"tcp_pool\": {},\n  \"peak_scheduled_events\": {},\n  \"total_events_processed\": {},\n  \"drops\": {},\n  \"duplicates\": {},\n  \"parse_errors\": {}\n}}",
        report.scenario_name,
        report.device_count,
        duration_s,
        register_tps,
        keepalive_tps,
        message_tps,
        report.resources.shard_count,
        report.resources.udp_sockets,
        report.resources.tcp_pool,
        report.resources.peak_scheduled_events,
        report.resources.total_events_processed,
        report.fault_counts.dropped,
        report.fault_counts.duplicated,
        report.message_counts.parse_errors,
    );
}

#[test]
#[ignore = "manual capacity test"]
fn capacity_dev_scale_converges_within_bounded_resources() {
    let scenario = load_capacity();
    let device_count = u64::from(scenario.device_count);
    let shards = scenario.shards;
    let udp_sockets = scenario.udp_sockets;
    let tcp_pool = scenario.tcp_pool;

    let report = run_scenario(scenario);
    print_capacity_metrics(&report);

    // Full convergence at scale.
    assert_eq!(
        report.outcomes.devices_registered, device_count,
        "all synthetic devices must register at capacity"
    );
    assert!(report.outcomes.keepalives_acked >= device_count);
    assert!(report.outcomes.catalog_received >= 1);
    assert!(report.outcomes.invites_answered >= 1);
    assert!(report.outcomes.byes_answered >= 1);
    assert_eq!(report.message_counts.parse_errors, 0);

    // Bounded resources: fixed pools, no per-device task/timer explosion.
    assert_eq!(report.resources.shard_count, shards);
    assert_eq!(report.resources.udp_sockets, udp_sockets);
    assert_eq!(report.resources.tcp_pool, tcp_pool);
    assert!(
        report.resources.peak_scheduled_events <= device_count.saturating_mul(4),
        "scheduled-event backlog must scale with devices, not grow unbounded (peak={}, devices={})",
        report.resources.peak_scheduled_events,
        device_count
    );
}

#[test]
#[ignore = "manual capacity test"]
fn capacity_run_is_reproducible() {
    let a = run_scenario(load_capacity());
    let b = run_scenario(load_capacity());
    assert_eq!(a.transcript_hash, b.transcript_hash);
    assert_eq!(a.outcomes, b.outcomes);
    assert_eq!(a.resources, b.resources);
}
