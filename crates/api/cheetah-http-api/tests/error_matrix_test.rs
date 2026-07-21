//! PROD-API-002 error matrix: stable Problem Details for common HTTP statuses.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{TestServer, TestServerOptions};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    aud: String,
    iss: String,
}

fn fixture_pem(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("read fixture pem")
}

fn sign_jwt(claims: &TestClaims) -> String {
    let private_pem = fixture_pem("jwt_private.pem");
    let key = EncodingKey::from_rsa_pem(private_pem.as_bytes()).expect("private key");
    encode(&Header::new(Algorithm::RS256), claims, &key).expect("sign jwt")
}

fn assert_problem(status: reqwest::StatusCode, body: &serde_json::Value, code: &str) {
    assert_eq!(body["code"], code, "body={body}");
    assert_eq!(body["status"], status.as_u16());
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(body["request_id"].as_str().is_some());
}

#[tokio::test]
async fn unauthenticated_returns_401_problem_details() {
    let server = TestServer::new().await;
    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("x-request-id", "019f8400-0000-7000-8000-000000000001")
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_problem(reqwest::StatusCode::UNAUTHORIZED, &body, "UNAUTHENTICATED");
    assert_eq!(body["request_id"], "019f8400-0000-7000-8000-000000000001");
}

#[tokio::test]
async fn missing_resource_returns_404_problem_details() {
    let server = TestServer::new().await;
    let missing = uuid::Uuid::now_v7();
    let response = server
        .request(reqwest::Method::GET, &format!("/api/v1/devices/{missing}"))
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_problem(reqwest::StatusCode::NOT_FOUND, &body, "NOT_FOUND");
}

#[tokio::test]
async fn missing_idempotency_key_returns_400() {
    let server = TestServer::new().await;
    let response = server
        .request(reqwest::Method::POST, "/api/v1/operations")
        .json(&serde_json::json!({
            "device_id": uuid::Uuid::now_v7().to_string(),
            "target": {
                "tenant_id": server.tenant_id(),
                "kind": "device",
                "id": { "device": uuid::Uuid::now_v7().to_string() }
            },
            "payload": {
                "Ptz": {
                    "channel_id": uuid::Uuid::now_v7().to_string(),
                    "direction": "right",
                    "speed": 0.5
                }
            },
            "idempotency_key": "ignored",
            "deadline": null,
            "expected_owner_epoch": 0
        }))
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_problem(reqwest::StatusCode::BAD_REQUEST, &body, "INVALID_ARGUMENT");
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("Idempotency-Key"),
        "message={}",
        body["message"]
    );
}

#[tokio::test]
async fn rate_limit_returns_429_problem_details() {
    let server = TestServer::with_options(TestServerOptions {
        rate_limit_requests_per_second: 1,
        rate_limit_burst: 1,
        ..Default::default()
    })
    .await;

    let first = server
        .request(reqwest::Method::GET, "/api/v1/devices")
        .send()
        .await
        .expect("first");
    assert_eq!(first.status(), reqwest::StatusCode::OK);

    let second = server
        .request(reqwest::Method::GET, "/api/v1/devices")
        .header("x-request-id", "019f8400-0000-7000-8000-000000000002")
        .send()
        .await
        .expect("second");
    assert_eq!(second.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    let body: serde_json::Value = second.json().await.expect("json");
    assert_problem(
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        &body,
        "RATE_LIMITED",
    );
}

#[tokio::test]
async fn jwt_tenant_mismatch_returns_403() {
    let public_pem = fixture_pem("jwt_public.pem");
    let tenant_a = uuid::Uuid::now_v7().to_string();
    let tenant_b = uuid::Uuid::now_v7().to_string();
    let server = TestServer::with_options(TestServerOptions {
        disable_static_api_key: true,
        jwt_public_key_pem: Some(public_pem),
        jwt_audience: vec!["cheetah-signaling".to_string()],
        jwt_issuer: vec!["cheetah-test".to_string()],
        ..Default::default()
    })
    .await;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = sign_jwt(&TestClaims {
        sub: "viewer-user".to_string(),
        exp: now + 3600,
        tenant_id: Some(tenant_a.clone()),
        scope: Some("viewer".to_string()),
        aud: "cheetah-signaling".to_string(),
        iss: "cheetah-test".to_string(),
    });

    let response = server
        .request_unauthenticated(reqwest::Method::GET, "/api/v1/devices")
        .header("Authorization", format!("Bearer {token}"))
        .header("x-tenant-id", &tenant_b)
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_problem(reqwest::StatusCode::FORBIDDEN, &body, "PERMISSION_DENIED");
}

#[tokio::test]
async fn jwt_missing_scope_returns_403() {
    let public_pem = fixture_pem("jwt_public.pem");
    let tenant = uuid::Uuid::now_v7().to_string();
    let server = TestServer::with_options(TestServerOptions {
        disable_static_api_key: true,
        jwt_public_key_pem: Some(public_pem),
        jwt_audience: vec!["cheetah-signaling".to_string()],
        jwt_issuer: vec!["cheetah-test".to_string()],
        ..Default::default()
    })
    .await;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let token = sign_jwt(&TestClaims {
        sub: "limited-user".to_string(),
        exp: now + 3600,
        tenant_id: Some(tenant.clone()),
        scope: Some("viewer".to_string()),
        aud: "cheetah-signaling".to_string(),
        iss: "cheetah-test".to_string(),
    });

    let response = server
        .request_unauthenticated(reqwest::Method::POST, "/api/v1/devices")
        .header("Authorization", format!("Bearer {token}"))
        .header("x-tenant-id", &tenant)
        .json(&serde_json::json!({
            "protocol": "onvif",
            "external_id": "scope-test",
            "authority": "test",
            "name": "Scope Test",
            "kind": "camera",
        }))
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_problem(reqwest::StatusCode::FORBIDDEN, &body, "PERMISSION_DENIED");
}

#[tokio::test]
async fn fallback_unknown_route_returns_404_problem_details() {
    let server = TestServer::new().await;
    let response = server
        .request(reqwest::Method::GET, "/api/v1/does-not-exist")
        .send()
        .await
        .expect("send");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: serde_json::Value = response.json().await.expect("json");
    assert_eq!(body["code"], "NOT_FOUND");
    assert_eq!(body["status"], 404);
}
