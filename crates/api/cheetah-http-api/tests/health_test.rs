//! Integration tests for health and readiness endpoints.

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
