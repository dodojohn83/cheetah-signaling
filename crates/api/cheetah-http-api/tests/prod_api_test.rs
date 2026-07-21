//! Integration tests for PROD-API-001 endpoints that were previously 501.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn create_and_list_tenants() {
    let server = common::TestServer::new().await;

    let create = server
        .request(reqwest::Method::POST, "/api/v1/tenants")
        .json(&serde_json::json!({ "name": "Acme Corp" }))
        .send()
        .await
        .expect("create tenant");
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let created: serde_json::Value = create.json().await.expect("body");
    assert_eq!(created["name"], "Acme Corp");
    let tenant_id = created["tenant_id"].as_str().expect("tenant_id");

    let list = server
        .request(reqwest::Method::GET, "/api/v1/tenants")
        .send()
        .await
        .expect("list tenants");
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let page: serde_json::Value = list.json().await.expect("list body");
    let items = page["items"].as_array().expect("items");
    assert!(
        items
            .iter()
            .any(|t| t["tenant_id"].as_str() == Some(tenant_id)),
        "created tenant must appear in list"
    );
}

#[tokio::test]
async fn list_nodes_returns_empty_page() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/api/v1/nodes")
        .send()
        .await
        .expect("list nodes");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let page: serde_json::Value = response.json().await.expect("body");
    assert!(page["items"].as_array().expect("items").is_empty());
}

#[tokio::test]
async fn create_operation_and_get() {
    let server = common::TestServer::new().await;

    let device_body = serde_json::json!({
        "protocol": "onvif",
        "external_id": "op-device-1",
        "authority": "test",
        "name": "Operation Device",
        "kind": "camera",
    });
    let device_resp = server
        .request(reqwest::Method::POST, "/api/v1/devices")
        .json(&device_body)
        .send()
        .await
        .expect("create device");
    assert_eq!(device_resp.status(), reqwest::StatusCode::CREATED);
    let device: serde_json::Value = device_resp.json().await.expect("device body");
    let device_id = device["device_id"].as_str().expect("device_id");
    let tenant_id = device["tenant_id"].as_str().expect("tenant_id");

    let channel_id = uuid::Uuid::now_v7().to_string();
    let op_body = serde_json::json!({
        "device_id": device_id,
        "target": {
            "tenant_id": tenant_id,
            "kind": "device",
            "id": { "device": device_id }
        },
        "payload": {
            "Ptz": {
                "channel_id": channel_id,
                "direction": "right",
                "speed": 0.5
            }
        },
        "idempotency_key": "will-be-overwritten",
        "deadline": null,
        "expected_owner_epoch": 0
    });

    let create = server
        .request(reqwest::Method::POST, "/api/v1/operations")
        .header("Idempotency-Key", "op-create-1")
        .json(&op_body)
        .send()
        .await
        .expect("create operation");
    if create.status() != reqwest::StatusCode::ACCEPTED {
        let text = create.text().await.unwrap_or_default();
        panic!("unexpected create status body: {text}");
    }
    let location = create
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .expect("Location header on 202");
    let created: serde_json::Value = create.json().await.expect("op body");
    let operation_id = created["operation_id"].as_str().expect("operation_id");
    assert_eq!(location, format!("/api/v1/operations/{operation_id}"));
    assert_eq!(created["status"], "pending");

    let get = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/operations/{operation_id}"),
        )
        .send()
        .await
        .expect("get operation");
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let fetched: serde_json::Value = get.json().await.expect("get body");
    assert_eq!(fetched["operation_id"], operation_id);
    assert_eq!(fetched["idempotency_key"], "op-create-1");
}

#[tokio::test]
async fn get_media_session_not_found() {
    let server = common::TestServer::new().await;
    let missing = uuid::Uuid::now_v7();
    let response = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/media/sessions/{missing}"),
        )
        .header("x-request-id", "019f835e-0000-7000-8000-000000000099")
        .send()
        .await
        .expect("get session");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok()),
        Some("019f835e-0000-7000-8000-000000000099")
    );
    let problem: serde_json::Value = response.json().await.expect("problem body");
    assert_eq!(
        problem["request_id"].as_str(),
        Some("019f835e-0000-7000-8000-000000000099")
    );
    assert!(!problem["code"].as_str().unwrap_or("").is_empty());
}

#[tokio::test]
async fn invalid_json_body_returns_problem_details() {
    let server = common::TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/tenants")
        .header("content-type", "application/json")
        .body("{not-json")
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let problem: serde_json::Value = response.json().await.expect("problem");
    assert_eq!(problem["code"], "INVALID_JSON");
    assert!(problem["request_id"].as_str().is_some());
}

#[tokio::test]
async fn device_update_requires_if_match_and_returns_412_on_mismatch() {
    let server = common::TestServer::new().await;
    let create = server
        .request(reqwest::Method::POST, "/api/v1/devices")
        .json(&serde_json::json!({
            "protocol": "onvif",
            "external_id": "rev-device-1",
            "authority": "test",
            "name": "Revision Device",
            "kind": "camera",
        }))
        .send()
        .await
        .expect("create");
    assert_eq!(create.status(), reqwest::StatusCode::CREATED);
    let created: serde_json::Value = create.json().await.expect("body");
    let device_id = created["device_id"].as_str().expect("device_id");
    let revision = created["revision"].as_u64().expect("revision");

    let missing = server
        .request(
            reqwest::Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
        )
        .json(&serde_json::json!({
            "capabilities": [{"key": "ptz", "value": {"boolean": true}}],
            "metadata": null
        }))
        .send()
        .await
        .expect("patch without if-match");
    assert_eq!(missing.status(), reqwest::StatusCode::BAD_REQUEST);

    let stale = server
        .request(
            reqwest::Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
        )
        .header("If-Match", format!("\"{}\"", revision.saturating_add(99)))
        .json(&serde_json::json!({
            "capabilities": [{"key": "ptz", "value": {"boolean": true}}],
            "metadata": null
        }))
        .send()
        .await
        .expect("patch stale");
    if stale.status() != reqwest::StatusCode::PRECONDITION_FAILED {
        let text = stale.text().await.unwrap_or_default();
        panic!("expected 412, body: {text}");
    }
    let problem: serde_json::Value = stale.json().await.expect("problem");
    assert_eq!(problem["code"], "FAILED_PRECONDITION");

    let ok = server
        .request(
            reqwest::Method::PATCH,
            &format!("/api/v1/devices/{device_id}"),
        )
        .header("If-Match", format!("\"{revision}\""))
        .json(&serde_json::json!({
            "capabilities": [{"key": "ptz", "value": {"boolean": true}}],
            "metadata": null
        }))
        .send()
        .await
        .expect("patch ok");
    if ok.status() != reqwest::StatusCode::OK {
        let text = ok.text().await.unwrap_or_default();
        panic!("expected 200, body: {text}");
    }
    assert!(ok.headers().get("etag").is_some());
}
