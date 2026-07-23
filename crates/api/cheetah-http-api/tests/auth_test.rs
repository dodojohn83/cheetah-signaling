//! Integration tests for authentication and authorization.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn unauthenticated_request_returns_401() {
    let server = common::TestServer::new().await;
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn invalid_api_key_returns_401() {
    let server = common::TestServer::new().await;
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("Authorization", "Bearer invalid-key")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn valid_api_key_allows_authenticated_request() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/metrics")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn api_key_fallback_ignores_non_bearer_authorization() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/metrics")
        .header("Authorization", "Basic dXNlcjpwYXNz")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn non_utf8_authorization_returns_401() {
    let server = common::TestServer::new().await;
    let value = reqwest::header::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap();
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("Authorization", value)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_utf8_authorization_does_not_fall_back_to_api_key() {
    let server = common::TestServer::new().await;
    let value = reqwest::header::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap();
    let response = server
        .request(reqwest::Method::GET, "/metrics")
        .header("Authorization", value)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn non_utf8_x_api_key_returns_401() {
    let server = common::TestServer::new().await;
    let value = reqwest::header::HeaderValue::from_bytes(&[0xff, 0xfe]).unwrap();
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("x-api-key", value)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_authorization_header_returns_401() {
    let server = common::TestServer::new().await;
    let oversized = format!("Bearer {}", "x".repeat(16_384));
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("Authorization", oversized)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_x_api_key_header_returns_401() {
    let server = common::TestServer::new().await;
    let oversized = "x".repeat(4_097);
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("x-api-key", oversized)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn oversized_x_tenant_id_header_returns_400() {
    let server = common::TestServer::new().await;
    let oversized = "x".repeat(129);
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("x-api-key", server.api_key())
        .header("x-tenant-id", oversized)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
