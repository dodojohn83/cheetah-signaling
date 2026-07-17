//! Integration tests for health, readiness and metrics endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn live_returns_ok() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/health/live")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn ready_returns_ok_after_migrations() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/health/ready")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["status"], "ready");
}

#[tokio::test]
async fn metrics_returns_counters() {
    let server = common::TestServer::new().await;

    // Generate a 2xx and a 4xx response so status-family counters are exercised.
    let live = server
        .request(reqwest::Method::GET, "/health/live")
        .send()
        .await
        .expect("send request");
    assert_eq!(live.status(), reqwest::StatusCode::OK);

    let _not_found = server
        .request_unauthenticated(reqwest::Method::GET, "/not-a-route")
        .send()
        .await
        .expect("send request");

    let metrics = server
        .request(reqwest::Method::GET, "/metrics")
        .send()
        .await
        .expect("send request");
    assert_eq!(metrics.status(), reqwest::StatusCode::OK);
    let body = metrics.text().await.expect("read metrics body");
    assert!(body.contains("cheetah_http_requests_total"));
    assert!(body.contains("cheetah_http_responses_2xx_total"));
    assert!(body.contains("cheetah_http_responses_4xx_total"));
}
