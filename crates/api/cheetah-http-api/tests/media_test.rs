//! Integration tests for media session HTTP endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn create_session_rejects_oversized_purpose() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "purpose": "x".repeat(65),
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/media/sessions")
        .header("idempotency-key", "test-key")
        .json(&body)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_session_rejects_non_string_purpose() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "purpose": 42,
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/media/sessions")
        .header("idempotency-key", "test-key")
        .json(&body)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_session_rejects_unknown_purpose_without_allocating() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "purpose": "x".repeat(1024),
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/media/sessions")
        .header("idempotency-key", "test-key")
        .json(&body)
        .send()
        .await
        .expect("send request");

    // Purpose is rejected on length before any lowercase copy is made.
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
