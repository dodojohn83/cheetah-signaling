#![allow(clippy::unwrap_used, clippy::expect_used)]
use super::*;
use std::time::Duration;

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn per_device_concurrency_limits_concurrent_calls() {
    let config = DriverConfig {
        per_device_concurrency: 1,
        ..Default::default()
    };
    let driver = OnvifHttpDriver::new(&config).expect("driver should build");
    let endpoint = "http://example.com/onvif/device_service";

    let _first = driver
        .acquire_device_permit(endpoint, None)
        .await
        .expect("first permit should be available");

    let result = driver
        .acquire_device_permit(endpoint, Some(Duration::from_nanos(0)))
        .await;
    assert!(
        matches!(result, Err(DriverError::Timeout(_))),
        "second caller should be denied while the first permit is held, got {result:?}"
    );
}

#[tokio::test]
async fn idle_device_permits_are_evicted_when_map_exceeds_capacity() {
    let config = DriverConfig {
        per_device_concurrency: 1,
        max_tracked_device_endpoints: 1,
        ..Default::default()
    };
    let driver = OnvifHttpDriver::new(&config).expect("driver should build");
    let first_endpoint = "http://a.onvif/device_service";
    let second_endpoint = "http://b.onvif/device_service";

    let first = driver
        .acquire_device_permit(first_endpoint, None)
        .await
        .expect("first permit should be available");
    drop(first);

    let _second = driver
        .acquire_device_permit(second_endpoint, None)
        .await
        .expect("second permit should be available");

    let tracked = driver.device_permits.lock().expect("lock map").len();
    assert_eq!(
        tracked, 1,
        "idle first entry should be evicted to keep map bounded"
    );
}
