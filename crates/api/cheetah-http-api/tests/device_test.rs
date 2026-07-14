//! Integration tests for device HTTP endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn create_and_get_device_round_trip() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "protocol": "onvif",
        "external_id": "integration-test-1",
        "authority": "test",
        "name": "Integration Test Camera",
        "kind": "camera",
        "capabilities": null,
        "metadata": null,
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/devices")
        .json(&body)
        .send()
        .await
        .expect("send request");
    let status = response.status();
    if status != reqwest::StatusCode::CREATED {
        let text = response.text().await.unwrap_or_default();
        panic!("unexpected create status {} body: {}", status, text);
    }

    let created: serde_json::Value = response.json().await.expect("read body");
    let device_id = created["device_id"]
        .as_str()
        .expect("device_id in response");

    let get_response = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/devices/{}", device_id),
        )
        .send()
        .await
        .expect("send get request");
    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let fetched: serde_json::Value = get_response.json().await.expect("read body");
    assert_eq!(fetched["external_id"], "integration-test-1");
    assert_eq!(fetched["tenant_id"], server.tenant_id());
}

#[tokio::test]
async fn invalid_protocol_returns_400() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "protocol": "not-a-protocol",
        "external_id": "bad",
        "authority": "test",
        "name": "Bad",
        "kind": "camera",
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/devices")
        .json(&body)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn cross_tenant_device_is_not_visible() {
    let server = common::TestServer::new().await;
    let body = serde_json::json!({
        "protocol": "onvif",
        "external_id": "tenant-isolation-test",
        "authority": "test",
        "name": "Tenant Isolation Camera",
        "kind": "camera",
    });

    let response = server
        .request(reqwest::Method::POST, "/api/v1/devices")
        .json(&body)
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let created: serde_json::Value = response.json().await.expect("read body");
    let device_id = created["device_id"].as_str().expect("device_id");

    let other_tenant = uuid::Uuid::now_v7().to_string();
    let other_get = server
        .request_unauthenticated(
            reqwest::Method::GET,
            &format!("/api/v1/devices/{}", device_id),
        )
        .header("x-api-key", server.api_key())
        .header("x-tenant-id", other_tenant)
        .send()
        .await
        .expect("send get request");
    assert_eq!(other_get.status(), reqwest::StatusCode::NOT_FOUND);
}
