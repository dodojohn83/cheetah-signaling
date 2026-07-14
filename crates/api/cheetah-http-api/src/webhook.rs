//! Webhook HTTP client and background delivery worker.

use cheetah_signal_application::{
    WebhookHttpClient, WebhookHttpRequest, WebhookHttpResponse, WebhookService,
};
use cheetah_signal_types::{SignalError, SignalErrorKind};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::redirect::Policy;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// Outbound webhook HTTP client backed by `reqwest` with DNS-based SSRF checks.
#[derive(Clone, Debug)]
pub struct ReqwestWebhookClient {
    client: reqwest::Client,
}

impl ReqwestWebhookClient {
    /// Creates a new webhook client with redirects disabled and a
    /// DNS resolver that filters disallowed addresses so the resolved IP cannot
    /// change between validation and the actual request.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .redirect(Policy::none())
            .dns_resolver(FilteringResolver)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

#[derive(Clone, Debug)]
struct FilteringResolver;

impl Resolve for FilteringResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let iter = tokio::net::lookup_host((name.as_str(), 0))
                .await
                .map_err(|e| Box::new(e) as BoxError)?;
            let addrs: Vec<SocketAddr> = iter
                .filter(|addr| !is_disallowed_ip(&addr.ip()))
                .map(|addr| SocketAddr::new(addr.ip(), 0))
                .collect();
            if addrs.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    "no allowed addresses for host",
                )) as BoxError);
            }
            Ok(Box::new(addrs.into_iter()) as Addrs)
        })
    }
}

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

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
        IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        IpAddr::V6(v6) => is_disallowed_ipv6(v6),
    }
}

fn is_disallowed_ipv4(v4: &std::net::Ipv4Addr) -> bool {
    v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_private()
        || v4.is_broadcast()
        || v4.is_documentation()
        || v4.is_unspecified()
}

fn is_disallowed_ipv6(v6: &std::net::Ipv6Addr) -> bool {
    if v6.is_loopback()
        || v6.is_multicast()
        || v6.is_unspecified()
        || v6.is_unicast_link_local()
        || v6.is_unique_local()
    {
        return true;
    }
    if let Some(v4) = v6.to_ipv4() {
        return is_disallowed_ipv4(&v4);
    }
    false
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
