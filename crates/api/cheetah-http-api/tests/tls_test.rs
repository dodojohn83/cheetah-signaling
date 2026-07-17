//! Integration tests for HTTPS termination.

#![allow(clippy::unwrap_used)]

mod common;

#[tokio::test]
async fn tls_server_responds_to_health_check() {
    let server = common::TestServer::new_tls().await;

    let response = server
        .request(reqwest::Method::GET, "/health/live")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), reqwest::StatusCode::OK);
}
