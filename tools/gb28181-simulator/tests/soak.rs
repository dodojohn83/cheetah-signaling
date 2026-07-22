//! GB4-SYS-008: bounded endurance / soak harness.
//!
//! The simulator runs in virtual time, so a long soak window is simulated
//! deterministically without a wall-clock 24h/72h run. The core soak invariant
//! is *no monotonic growth*: the steady-state resource footprint (peak
//! scheduled timer-wheel events, fixed shard/socket/pool counts) must depend on
//! the concurrent device population and cadence, not on how long the window
//! runs. This test scales `duration_ms` up and asserts the peak scheduled-event
//! backlog stays flat -- a deterministic leak detector for timers/objects/work.
//!
//! Duration and device count are overridable via `SOAK_DEVICES` and
//! `SOAK_BASE_DURATION_MS` for extended release-candidate runs, but the default
//! development window keeps the test fast.
//!
//! Marked `#[ignore]` so it runs only on demand:
//! `cargo test -p cheetah-gb28181-simulator --test soak -- --ignored`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use cheetah_gb28181_simulator::{Scenario, run_scenario};

fn base_scenario() -> Scenario {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/scenarios/soak-dev.toml");
    let mut scenario = Scenario::from_toml_path(path).expect("soak-dev.toml parses");
    if let Ok(devices) = std::env::var("SOAK_DEVICES")
        && let Ok(parsed) = devices.parse::<u32>()
        && parsed > 0
    {
        scenario.device_count = parsed;
    }
    if let Ok(duration) = std::env::var("SOAK_BASE_DURATION_MS")
        && let Ok(parsed) = duration.parse::<u64>()
        && parsed > 0
    {
        scenario.duration_ms = parsed;
    }
    scenario
}

#[test]
#[ignore = "manual soak test"]
fn soak_footprint_does_not_grow_with_window() {
    let base = base_scenario();
    let base_duration = base.duration_ms;
    let device_count = u64::from(base.device_count);

    // Simulate progressively longer soak windows over the same population.
    let mut peaks = Vec::new();
    for multiplier in [1u64, 2, 4] {
        let mut scenario = base_scenario();
        scenario.duration_ms = base_duration.saturating_mul(multiplier);
        let report = run_scenario(scenario);
        eprintln!(
            "SOAK {{ \"window_ms\": {}, \"peak_scheduled_events\": {}, \"total_events\": {}, \"registered\": {}, \"parse_errors\": {} }}",
            base_duration.saturating_mul(multiplier),
            report.resources.peak_scheduled_events,
            report.resources.total_events_processed,
            report.outcomes.devices_registered,
            report.message_counts.parse_errors,
        );

        // Steady-state invariants at every window length.
        assert_eq!(report.outcomes.devices_registered, device_count);
        assert_eq!(report.message_counts.parse_errors, 0);
        assert_eq!(report.resources.shard_count, base.shards);

        peaks.push(report.resources.peak_scheduled_events);
    }

    // No monotonic growth: a 2x/4x longer window must not raise the peak
    // in-flight backlog. Bounded by the concurrent population, not the window.
    let baseline_peak = peaks[0];
    for (idx, peak) in peaks.iter().enumerate() {
        assert!(
            *peak <= baseline_peak,
            "soak window {}x raised peak scheduled events ({} > {}), indicating a leak",
            1u64 << idx,
            peak,
            baseline_peak
        );
    }
}

#[test]
#[ignore = "manual soak test"]
fn soak_longer_window_processes_more_events_but_stays_bounded() {
    // A longer window must process strictly more timer-wheel events (the soak
    // keeps making progress) while the *peak* backlog stays bounded.
    let short = base_scenario();
    let base_duration = short.duration_ms;
    let short_report = run_scenario(short);

    let mut long = base_scenario();
    long.duration_ms = base_duration.saturating_mul(4);
    let long_report = run_scenario(long);

    assert!(
        long_report.resources.total_events_processed
            > short_report.resources.total_events_processed,
        "a longer soak must keep making progress"
    );
    assert!(
        long_report.resources.peak_scheduled_events <= short_report.resources.peak_scheduled_events,
        "longer soak must not grow the peak in-flight backlog"
    );
}
