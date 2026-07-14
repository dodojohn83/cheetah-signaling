//! Integration tests for authentication and authorization.

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
async fn valid_api_key_allows_list_request() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/api/v1/devices")
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response.json::<serde_json::Value>().await.expect("read body");
    assert_eq!(body["items"], serde_json::json!([]));
}
