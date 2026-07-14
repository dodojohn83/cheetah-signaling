//! Webhook HTTP endpoint integration tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

pub mod common;

use common::TestServer;

#[tokio::test(flavor = "current_thread")]
async fn openapi_serves_valid_yaml_and_json() {
    let server = TestServer::new().await;

    let yaml = server
        .request(reqwest::Method::GET, "/api/v1/openapi.yaml")
        .send()
        .await
        .unwrap();
    assert_eq!(yaml.status(), 200);
    let yaml_text = yaml.text().await.unwrap();
    assert!(yaml_text.contains("openapi: 3.1.0"));

    let json = server
        .request(reqwest::Method::GET, "/api/v1/openapi.json")
        .send()
        .await
        .unwrap();
    assert_eq!(json.status(), 200);
    let json_text = json.text().await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&json_text).unwrap();
    assert!(value["paths"]["/api/v1/webhooks"].is_object());
}

#[tokio::test(flavor = "current_thread")]
async fn webhook_crud_and_validation() {
    let server = TestServer::new().await;

    // Creating a webhook with a disallowed URL is rejected.
    let bad = server
        .request(reqwest::Method::POST, "/api/v1/webhooks")
        .json(&serde_json::json!({
            "url": "http://localhost/webhook",
            "secret_ref": "sig.test",
            "event_types": ["device.online"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);

    // Create a valid webhook.
    let create = server
        .request(reqwest::Method::POST, "/api/v1/webhooks")
        .json(&serde_json::json!({
            "url": "http://example.com/webhook",
            "secret_ref": "sig.test",
            "event_types": ["device.online"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 201);
    let config: serde_json::Value = create.json().await.unwrap();
    let webhook_id = config["webhook_id"].as_str().unwrap().to_string();
    assert_eq!(config["url"], "http://example.com/webhook");
    assert!(config["enabled"].as_bool().unwrap());

    // List webhooks.
    let list = server
        .request(reqwest::Method::GET, "/api/v1/webhooks")
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);

    // Get webhook.
    let get = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/webhooks/{webhook_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), 200);

    // Update webhook.
    let update = server
        .request(
            reqwest::Method::PATCH,
            &format!("/api/v1/webhooks/{webhook_id}"),
        )
        .json(&serde_json::json!({
            "url": "http://example.com/webhook-v2",
            "event_types": ["device.offline"],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update.status(), 200);
    let updated: serde_json::Value = update.json().await.unwrap();
    assert_eq!(updated["url"], "http://example.com/webhook-v2");

    // Trigger a manual delivery.
    let trigger = server
        .request(
            reqwest::Method::POST,
            &format!("/api/v1/webhooks/{webhook_id}/deliveries"),
        )
        .json(&serde_json::json!({
            "event_type": "device.offline",
            "payload": {"device_id": "123"},
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(trigger.status(), 201);
    let delivery: serde_json::Value = trigger.json().await.unwrap();
    assert_eq!(delivery["status"], "pending");
    assert!(!delivery["signature"].as_str().unwrap().is_empty());

    // List deliveries.
    let deliveries = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/webhooks/{webhook_id}/deliveries"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(deliveries.status(), 200);
    let deliveries: serde_json::Value = deliveries.json().await.unwrap();
    assert_eq!(deliveries["items"].as_array().unwrap().len(), 1);

    // Delete webhook.
    let delete = server
        .request(
            reqwest::Method::DELETE,
            &format!("/api/v1/webhooks/{webhook_id}"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(delete.status(), 204);

    let list = server
        .request(reqwest::Method::GET, "/api/v1/webhooks")
        .send()
        .await
        .unwrap();
    let list: serde_json::Value = list.json().await.unwrap();
    assert!(list["items"].as_array().unwrap().is_empty());
}
