//! Integration tests for administrative operations endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn validate_config_accepts_default() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/validate-config")
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["valid"], true);
}

#[tokio::test]
async fn validate_config_rejects_zero_worker_threads() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/validate-config")
        .json(&serde_json::json!({
            "runtime": { "worker_threads": 0 }
        }))
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn db_status_returns_current() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/api/v1/admin/db-status")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["status"], "current");
}

#[tokio::test]
async fn db_migrate_returns_current() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/db-migrate")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["status"], "current");
}

#[tokio::test]
async fn node_drain_accepts_request() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/node-drain")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["draining"], true);
}

#[tokio::test]
async fn device_diagnostics_not_found() {
    let server = common::TestServer::new().await;
    let id = uuid::Uuid::now_v7().to_string();
    let response = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/admin/devices/{id}/diagnostics"),
        )
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn outbox_replay_reports_zero_when_empty() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/outbox-replay")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<serde_json::Value>()
        .await
        .expect("read body");
    assert_eq!(body["replayed"], 0);
}

#[tokio::test]
async fn reconcile_returns_not_implemented() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/admin/reconcile")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn admin_endpoints_require_authentication() {
    let server = common::TestServer::new().await;
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/admin/db-status")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}
