//! PERF-003: storm and backpressure scenario.
//!
//! Simulates a reconnect spike (10 % and 50 % of devices) against a PostgreSQL
//! backend with a small connection pool. The scenario is marked `#[ignore]` and
//! must be run manually.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator, request_context};
use cheetah_signal_application::{DeviceService, MarkDeviceOnlineRequest, RegisterDeviceRequest};
use cheetah_signal_types::{Clock, IdGenerator};
use cheetah_storage_api::Storage;
use cheetah_storage_postgres::PostgresStorage;
use testcontainers_modules::postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

mod perf_common;

const DEVICE_COUNT: usize = 200;
const RECONNECT_FRACTION: f64 = 0.5;
const CONCURRENCY: usize = 50;

async fn wait_for_postgres_ready(
    url: &str,
    timeout: Duration,
) -> Result<PostgresStorage, cheetah_storage_api::StorageError> {
    let started = std::time::Instant::now();
    loop {
        match PostgresStorage::new(url).await {
            Ok(storage) => return Ok(storage),
            Err(e) if started.elapsed() >= timeout => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

async fn setup_storm() -> (
    Arc<PostgresStorage>,
    Arc<dyn Clock>,
    Arc<InMemoryIdGenerator>,
    DeviceService,
) {
    let pg_container = postgres::Postgres::default().start().await.unwrap();
    let pg_port = pg_container.get_host_port_ipv4(5432).await.unwrap();
    let pg_host = pg_container.get_host().await.unwrap().to_string();
    let pg_url =
        format!("postgres://postgres:postgres@{pg_host}:{pg_port}/postgres?sslmode=disable");

    let storage = Arc::new(
        wait_for_postgres_ready(&pg_url, Duration::from_secs(30))
            .await
            .unwrap(),
    );
    storage.migration().run().await.unwrap();

    let clock: Arc<dyn Clock> = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let id_gen_dyn: Arc<dyn IdGenerator> = id_generator.clone();

    let device_service = DeviceService::new(clock.clone(), id_gen_dyn);

    (storage, clock, id_generator, device_service)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual performance test"]
async fn perf_storm_reconnect_spike() {
    let (storage, clock, id_generator, device_service) = setup_storm().await;

    let tenant = id_generator.generate_tenant_id();
    let ctx = request_context(tenant, id_generator.as_ref(), clock.as_ref());

    // Pre-register a fleet of devices.
    let mut device_ids = Vec::with_capacity(DEVICE_COUNT);
    for _ in 0..DEVICE_COUNT {
        let mut uow = storage.begin().await.unwrap();
        let device = device_service
            .register_or_update_device(
                &ctx,
                &mut *uow,
                RegisterDeviceRequest {
                    protocol: "gb28181".to_string(),
                    external_id: format!("perf-{}", id_generator.generate_device_id()),
                    authority: Some("auth".to_string()),
                    name: "storm-camera".to_string(),
                    kind: "camera".to_string(),
                    capabilities: None,
                    metadata: None,
                },
            )
            .await
            .unwrap()
            .device;
        device_ids.push(device.device_id);
    }

    let reconnect_count = (DEVICE_COUNT as f64 * RECONNECT_FRACTION) as usize;
    let reconnect_ids: Arc<Vec<_>> =
        Arc::new(device_ids.into_iter().take(reconnect_count).collect());
    let index = Arc::new(AtomicUsize::new(0));

    let storm_ctx = ctx.clone();
    let storm_storage = storage.clone();
    let storm_device_service = device_service.clone();
    let summary = perf_common::measure_concurrent(
        "storm_reconnect_spike",
        CONCURRENCY,
        reconnect_count / CONCURRENCY,
        move || {
            let ctx = storm_ctx.clone();
            let device_service = storm_device_service.clone();
            let storage = storm_storage.clone();
            let ids = reconnect_ids.clone();
            let index = index.clone();
            async move {
                let idx = index.fetch_add(1, Ordering::Relaxed) % ids.len();
                let device_id = ids[idx];
                let mut uow = storage.begin().await.unwrap();
                device_service
                    .mark_device_online(
                        &ctx,
                        &mut *uow,
                        device_id,
                        MarkDeviceOnlineRequest { reason: None },
                    )
                    .await
                    .unwrap();
            }
        },
    )
    .await;

    summary.print();
    assert!(
        summary.p95_ns < 100_000_000,
        "p95 reconnect heartbeat latency < 100ms under spike"
    );
    assert!(
        summary.throughput_per_sec > 100.0,
        "throughput must stay above 100 ops/sec under spike"
    );
}
