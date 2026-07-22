//! GB4-SYS-006: deterministic transport-chaos scenario driven by the
//! GB4-TST-004 fault DSL.
//!
//! Loads `scenarios/chaos-cluster.toml` and asserts the three properties a
//! chaos run must hold: determinism (identical transcript hash across repeated
//! runs), graceful degradation / convergence (a bounded, non-empty subset of
//! devices still register and complete the catalog/invite/bye path despite
//! loss, delay, duplication and 503s), and bounded resources (fixed shard
//! count, no per-device task/timer explosion, no unbounded parse errors).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_simulator::{Scenario, run_scenario};

fn load_chaos() -> Scenario {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios/chaos-cluster.toml");
    Scenario::from_toml_path(path).expect("chaos-cluster.toml parses")
}

#[test]
fn chaos_run_is_deterministic() {
    let a = run_scenario(load_chaos());
    let b = run_scenario(load_chaos());
    assert_eq!(
        a.transcript_hash, b.transcript_hash,
        "chaos run must be reproducible"
    );
    assert_eq!(a.message_counts, b.message_counts);
    assert_eq!(a.outcomes, b.outcomes);
    assert_eq!(a.fault_counts, b.fault_counts);
}

#[test]
fn chaos_faults_are_injected() {
    let report = run_scenario(load_chaos());
    assert!(report.fault_counts.dropped > 0, "drop fault must fire");
    assert!(report.fault_counts.delayed > 0, "delay fault must fire");
    assert!(
        report.fault_counts.duplicated > 0,
        "duplicate fault must fire"
    );
    assert!(
        report.fault_counts.sip_errors_injected > 0,
        "sip_error fault must fire"
    );
}

#[test]
fn chaos_converges_with_bounded_degradation() {
    let scenario = load_chaos();
    let device_count = u64::from(scenario.device_count);
    let report = run_scenario(scenario);

    // Convergence: despite loss, delay, duplication and intermittent 503s,
    // bounded retries drive every device to a registered steady state.
    assert_eq!(
        report.outcomes.devices_registered, device_count,
        "bounded retries must converge all devices despite chaos"
    );
    assert!(report.outcomes.keepalives_acked > 0);
    assert!(report.outcomes.catalog_received >= 1);
    assert!(report.outcomes.invites_answered >= 1);
    assert!(report.outcomes.byes_answered >= 1);

    // No malformed fault is configured, so the parser contract stays intact.
    assert_eq!(report.message_counts.parse_errors, 0);
}

#[test]
fn chaos_resources_stay_bounded() {
    let scenario = load_chaos();
    let shards = scenario.shards;
    let device_count = u64::from(scenario.device_count);
    let report = run_scenario(scenario);

    // Fixed shard workers: no per-device task.
    assert_eq!(report.resources.shard_count, shards);

    // Bounded scheduling: the peak number of concurrently scheduled events is
    // proportional to the device population, not unbounded backlog growth.
    assert!(report.resources.peak_scheduled_events > 0);
    assert!(
        report.resources.peak_scheduled_events <= device_count.saturating_mul(8),
        "scheduled-event backlog must stay bounded (peak={}, devices={})",
        report.resources.peak_scheduled_events,
        device_count
    );
}
