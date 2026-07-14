//! Webhook HTTP client and background delivery worker.

use cheetah_signal_application::{
    WebhookHttpClient, WebhookHttpRequest, WebhookHttpResponse, WebhookService,
};
use cheetah_signal_types::{SignalError, SignalErrorKind};
use reqwest::redirect::Policy;
use std::net::IpAddr;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Outbound webhook HTTP client backed by `reqwest` with DNS-based SSRF checks.
#[derive(Clone, Debug)]
pub struct ReqwestWebhookClient {
    client: reqwest::Client,
}

impl ReqwestWebhookClient {
    /// Creates a new webhook client with redirects disabled.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for ReqwestWebhookClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl WebhookHttpClient for ReqwestWebhookClient {
    async fn send(&self, request: WebhookHttpRequest) -> Result<WebhookHttpResponse, SignalError> {
        let url = url::Url::parse(&request.url)
            .map_err(|e| SignalError::new(SignalErrorKind::InvalidArgument, e.to_string()))?;

        validate_host(&url)?;

        if let Some(host) = url.host_str() {
            let port = url.port_or_known_default().unwrap_or(443);
            if host.parse::<IpAddr>().is_err() {
                let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
                    SignalError::new(
                        SignalErrorKind::Unavailable,
                        format!("dns lookup failed: {e}"),
                    )
                })?;
                for addr in addrs {
                    if is_disallowed_ip(&addr.ip()) {
                        return Err(SignalError::new(
                            SignalErrorKind::InvalidArgument,
                            "resolved webhook address is not allowed",
                        ));
                    }
                }
            }
        }

        let timeout_ms = request
            .timeout
            .map(|d| d.as_millis())
            .unwrap_or(30_000)
            .max(0) as u64;
        let timeout = Duration::from_millis(timeout_ms);

        let mut reqwest_headers = reqwest::header::HeaderMap::new();
        for (name, value) in request.headers {
            let key = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| SignalError::new(SignalErrorKind::InvalidArgument, e.to_string()))?;
            let val = reqwest::header::HeaderValue::from_str(&value)
                .map_err(|e| SignalError::new(SignalErrorKind::InvalidArgument, e.to_string()))?;
            reqwest_headers.insert(key, val);
        }

        let req = self
            .client
            .post(request.url)
            .headers(reqwest_headers)
            .body(request.body)
            .timeout(timeout);

        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                SignalError::new(SignalErrorKind::Timeout, "webhook request timed out")
            } else {
                SignalError::new(
                    SignalErrorKind::Unavailable,
                    format!("webhook request failed: {e}"),
                )
            }
        })?;

        let status = resp.status().as_u16();
        let body = match resp.bytes().await {
            Ok(bytes) => bytes.to_vec(),
            Err(_) => Vec::new(),
        };
        Ok(WebhookHttpResponse { status, body })
    }
}

fn validate_host(url: &url::Url) -> Result<(), SignalError> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "webhook url scheme must be http or https",
        ));
    }
    let host = url.host_str().ok_or_else(|| {
        SignalError::new(
            SignalErrorKind::InvalidArgument,
            "webhook url must have a host",
        )
    })?;

    if host.eq_ignore_ascii_case("localhost")
        || host.ends_with(".local")
        || host.eq_ignore_ascii_case("metadata")
        || host.eq_ignore_ascii_case("metadata.google.internal")
    {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "webhook url host is not allowed",
        ));
    }

    if let Ok(ip) = host.parse::<IpAddr>()
        && is_disallowed_ip(&ip)
    {
        return Err(SignalError::new(
            SignalErrorKind::InvalidArgument,
            "webhook url points to a disallowed address",
        ));
    }

    Ok(())
}

fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_private()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_multicast() || v6.is_unspecified(),
    }
}

/// Runs the webhook delivery worker until the cancellation token fires.
pub async fn run_delivery_worker(
    service: WebhookService,
    cancel: CancellationToken,
    interval: Duration,
) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => {
                if let Err(e) = service.process_pending(10).await {
                    tracing::error!("webhook delivery worker error: {e}");
                }
            }
        }
    }
}
