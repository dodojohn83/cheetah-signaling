//! Integration tests for paginated list endpoints.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn device_list_pagination_and_name_prefix() {
    let server = common::TestServer::new().await;

    for i in 0..3 {
        let body = serde_json::json!({
            "protocol": "onvif",
            "external_id": format!("list-test-{i}"),
            "authority": "test",
            "name": if i == 0 { "alpha camera".to_string() } else { format!("camera {i}") },
            "kind": "camera",
        });
        let response = server
            .request(reqwest::Method::POST, "/api/v1/devices")
            .json(&body)
            .send()
            .await
            .expect("send request");
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    let list = server
        .request(reqwest::Method::GET, "/api/v1/devices?page_size=2")
        .send()
        .await
        .expect("send list request");
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let page: serde_json::Value = list.json().await.expect("read body");
    assert_eq!(page["items"].as_array().unwrap().len(), 2);
    assert!(page["next_cursor"].as_str().is_some());

    let next_cursor = page["next_cursor"].as_str().unwrap();
    let list2 = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/devices?page_size=2&cursor={next_cursor}"),
        )
        .send()
        .await
        .expect("send list2 request");
    assert_eq!(list2.status(), reqwest::StatusCode::OK);
    let page2: serde_json::Value = list2.json().await.expect("read body");
    assert_eq!(page2["items"].as_array().unwrap().len(), 1);

    let prefix = server
        .request(
            reqwest::Method::GET,
            "/api/v1/devices?name_prefix=alpha&page_size=10",
        )
        .send()
        .await
        .expect("send prefix request");
    assert_eq!(prefix.status(), reqwest::StatusCode::OK);
    let prefix_page: serde_json::Value = prefix.json().await.expect("read body");
    assert_eq!(prefix_page["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn oversized_cursor_is_rejected() {
    let server = common::TestServer::new().await;
    let oversized_cursor = "x".repeat(1025);
    let response = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/devices?cursor={oversized_cursor}"),
        )
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oversized_name_prefix_is_rejected() {
    let server = common::TestServer::new().await;
    let oversized_prefix = "x".repeat(257);
    let response = server
        .request(
            reqwest::Method::GET,
            &format!("/api/v1/devices?name_prefix={oversized_prefix}"),
        )
        .send()
        .await
        .expect("send request");
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
}
