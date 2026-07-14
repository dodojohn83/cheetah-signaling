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
