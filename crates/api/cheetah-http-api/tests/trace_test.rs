//! Integration tests for W3C trace context propagation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

#[tokio::test]
async fn trace_context_is_echoed_in_response() {
    let server = common::TestServer::new().await;
    let traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let tracestate = "congo=t61rcWkgMzE";

    let response = server
        .request(reqwest::Method::GET, "/health/live")
        .header("traceparent", traceparent)
        .header("tracestate", tracestate)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("traceparent")
            .expect("traceparent header"),
        traceparent
    );
    assert_eq!(
        response
            .headers()
            .get("tracestate")
            .expect("tracestate header"),
        tracestate
    );
}

#[tokio::test]
async fn trace_context_is_echoed_on_404_response() {
    let server = common::TestServer::new().await;
    let traceparent = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let tracestate = "congo=t61rcWkgMzE";

    let response = server
        .request(reqwest::Method::GET, "/api/v1/not-a-real-endpoint")
        .header("traceparent", traceparent)
        .header("tracestate", tracestate)
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert_eq!(
        response
            .headers()
            .get("traceparent")
            .expect("traceparent header"),
        traceparent
    );
    assert_eq!(
        response
            .headers()
            .get("tracestate")
            .expect("tracestate header"),
        tracestate
    );
}
