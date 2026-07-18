//! Smoke test so `cargo nextest run --workspace` does not treat the perf crate
//! as having no tests. The real performance scenarios are `#[ignore]` and must
//! be run manually.

#[tokio::test]
async fn perf_harness_compiles_and_loads() {
    assert!(std::process::id() > 0);
}
