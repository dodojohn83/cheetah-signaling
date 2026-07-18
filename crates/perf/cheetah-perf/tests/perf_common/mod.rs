//! Shared helpers for the `cheetah-perf` scenarios.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    dead_code,
    unused_variables
)]

use std::future::Future;
use std::time::{Duration, Instant};

/// Simple per-iteration latency sample.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// Elapsed nanoseconds for this operation.
    pub ns: u64,
}

/// Scenario-level metrics summary.
#[derive(Debug)]
pub struct Summary {
    pub name: &'static str,
    pub iterations: usize,
    pub total_ns: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub p50_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub throughput_per_sec: f64,
}

impl Summary {
    pub fn print(&self) {
        eprintln!(
            "PERF {{
  \"name\": \"{}\",
  \"iterations\": {},
  \"total_ms\": {},
  \"min_ms\": {},
  \"max_ms\": {},
  \"p50_ms\": {},
  \"p95_ms\": {},
  \"p99_ms\": {},
  \"throughput_per_sec\": {:.2}
}}",
            self.name,
            self.iterations,
            self.total_ns / 1_000_000,
            self.min_ns / 1_000_000,
            self.max_ns / 1_000_000,
            self.p50_ns / 1_000_000,
            self.p95_ns / 1_000_000,
            self.p99_ns / 1_000_000,
            self.throughput_per_sec,
        );
    }
}

/// Measures `iterations` calls to `op` and returns a `Summary`.
/// `setup` is called once before the loop and `teardown` after. `op` is
/// awaited sequentially so the reported latency is end-to-end per operation.
pub async fn measure<Fut>(
    name: &'static str,
    iterations: usize,
    mut op: impl FnMut() -> Fut,
) -> Summary
where
    Fut: Future<Output = ()>,
{
    assert!(iterations > 0, "iterations must be > 0");
    let mut samples = Vec::with_capacity(iterations);
    let start = Instant::now();
    for _ in 0..iterations {
        let op_start = Instant::now();
        op().await;
        let ns = op_start.elapsed().as_nanos() as u64;
        samples.push(Sample { ns });
    }
    let total_ns = start.elapsed().as_nanos() as u64;
    summarize(name, iterations, total_ns, samples)
}

/// Measures parallel invocations of `op` by spawning `concurrency` tasks, each
/// executing `iterations_per_task` operations. The returned latency samples are
/// per-operation wall time for each individual task.
pub async fn measure_concurrent<Fut>(
    name: &'static str,
    concurrency: usize,
    iterations_per_task: usize,
    op: impl Fn() -> Fut + Send + Clone + 'static,
) -> Summary
where
    Fut: Future<Output = ()> + Send,
{
    assert!(concurrency > 0);
    assert!(iterations_per_task > 0);

    let mut handles = Vec::with_capacity(concurrency);
    let start = Instant::now();
    for _ in 0..concurrency {
        let op = op.clone();
        handles.push(tokio::spawn(async move {
            let mut samples = Vec::with_capacity(iterations_per_task);
            for _ in 0..iterations_per_task {
                let op_start = Instant::now();
                op().await;
                let ns = op_start.elapsed().as_nanos() as u64;
                samples.push(Sample { ns });
            }
            samples
        }));
    }

    let mut all_samples = Vec::with_capacity(concurrency * iterations_per_task);
    for handle in handles {
        all_samples.extend(handle.await.unwrap());
    }
    let total_ns = start.elapsed().as_nanos() as u64;
    summarize(
        name,
        concurrency * iterations_per_task,
        total_ns,
        all_samples,
    )
}

fn summarize(
    name: &'static str,
    iterations: usize,
    total_ns: u64,
    mut samples: Vec<Sample>,
) -> Summary {
    samples.sort_by_key(|s| s.ns);
    let min_ns = samples.first().unwrap().ns;
    let max_ns = samples.last().unwrap().ns;
    let p50 = percentile(&samples, 0.50);
    let p95 = percentile(&samples, 0.95);
    let p99 = percentile(&samples, 0.99);
    let throughput_per_sec = if total_ns > 0 {
        iterations as f64 * 1_000_000_000.0 / total_ns as f64
    } else {
        0.0
    };
    Summary {
        name,
        iterations,
        total_ns,
        min_ns,
        max_ns,
        p50_ns: p50,
        p95_ns: p95,
        p99_ns: p99,
        throughput_per_sec,
    }
}

fn percentile(sorted: &[Sample], p: f64) -> u64 {
    let index = ((sorted.len() as f64 - 1.0) * p).floor() as usize;
    sorted[index.min(sorted.len().saturating_sub(1))].ns
}

/// Returns the duration that has elapsed since a baseline instant, expressed
/// in seconds with millisecond precision. Used for startup-time and RSS-style
/// coarse measurements that are outside the per-operation latency path.
pub fn elapsed_secs(start: Instant) -> f64 {
    start.elapsed().as_secs_f64()
}

/// Sleeps for the given wall-clock duration without advancing a fake clock.
/// Only used between scenarios, never inside a measured operation.
pub async fn cooldown(d: Duration) {
    tokio::time::sleep(d).await;
}
