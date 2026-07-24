//! HTTP SOAP 1.2 client with SSRF checks, deadlines and body limits.

use crate::auth::{DeviceCredentials, inject_username_token};
use crate::config::DriverConfig;
use crate::error::{DriverError, DriverResult};
use bytes::BytesMut;
use cheetah_onvif_core::discovery::XAddrPolicy;
use cheetah_onvif_core::soap;
use futures::StreamExt;
use reqwest::{Client, StatusCode};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use url::Url;

/// SOAP-over-HTTP client for ONVIF service calls.
#[derive(Debug, Clone)]
pub struct SoapClient {
    client: Client,
    policy: XAddrPolicy,
    max_response_bytes: usize,
    request_timeout: Duration,
    permits: Arc<Semaphore>,
    follow_redirects: bool,
}

impl SoapClient {
    /// Creates a client from driver configuration.
    pub fn new(config: &DriverConfig) -> DriverResult<Self> {
        let client = Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(if config.follow_redirects {
                reqwest::redirect::Policy::custom(|attempt| {
                    // Default limited redirects; policy re-checked in post().
                    if attempt.previous().len() >= 3 {
                        attempt.stop()
                    } else {
                        attempt.follow()
                    }
                })
            } else {
                reqwest::redirect::Policy::none()
            })
            .user_agent("cheetah-onvif-driver/0.1")
            .build()
            .map_err(DriverError::http)?;

        Ok(Self {
            client,
            policy: config.xaddr_policy.clone(),
            max_response_bytes: config.max_response_bytes,
            request_timeout: config.request_timeout,
            permits: Arc::new(Semaphore::new(config.max_concurrent_requests.max(1))),
            follow_redirects: config.follow_redirects,
        })
    }

    /// Posts a SOAP envelope to `endpoint` and returns the response body.
    pub async fn post(
        &self,
        endpoint: &str,
        soap_action: &str,
        envelope_xml: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<String> {
        self.post_inner(endpoint, soap_action, envelope_xml, timeout)
            .await
    }

    /// Posts a SOAP envelope after injecting a WS-Security UsernameToken.
    pub async fn post_authenticated(
        &self,
        endpoint: &str,
        soap_action: &str,
        envelope_xml: &str,
        credentials: &DeviceCredentials,
        timeout: Option<Duration>,
    ) -> DriverResult<String> {
        let signed = inject_username_token(envelope_xml, credentials, None)?;
        self.post_inner(endpoint, soap_action, &signed, timeout)
            .await
    }

    async fn post_inner(
        &self,
        endpoint: &str,
        soap_action: &str,
        envelope_xml: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<String> {
        let url = Url::parse(endpoint)
            .map_err(|e| DriverError::Onvif(cheetah_onvif_core::OnvifError::invalid_xaddr(e)))?;
        self.policy.validate(&url).map_err(DriverError::Onvif)?;

        let overall_timeout = timeout.unwrap_or(self.request_timeout);
        let start = std::time::Instant::now();
        let _permit = tokio::time::timeout(overall_timeout, self.permits.acquire())
            .await
            .map_err(|_| DriverError::timeout("request permit wait timed out"))?
            .map_err(|_| DriverError::config("request semaphore closed"))?;
        let elapsed = start.elapsed();

        let request_timeout = timeout
            .map(|t| t.saturating_sub(elapsed))
            .unwrap_or_else(|| self.request_timeout.saturating_sub(elapsed));
        if request_timeout.is_zero() {
            return Err(DriverError::timeout("deadline exceeded after permit wait"));
        }
        let request = self
            .client
            .post(url.clone())
            .header("Content-Type", "application/soap+xml; charset=utf-8")
            .header("SOAPAction", format!("\"{soap_action}\""))
            .body(envelope_xml.to_string())
            .timeout(request_timeout);

        let response = request.send().await.map_err(|e| {
            if e.is_timeout() {
                DriverError::timeout(e)
            } else {
                DriverError::http(e)
            }
        })?;

        // Re-validate final URL after optional redirects.
        if self.follow_redirects {
            let final_url = response.url().clone();
            if final_url != url {
                self.policy
                    .validate_redirect(&url, &final_url)
                    .map_err(DriverError::Onvif)?;
            }
        }

        let status = response.status();

        // Enforce the configured response-size bound while streaming so a
        // misbehaving peer cannot make us allocate an unbounded buffer.
        let mut body_bytes = BytesMut::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(DriverError::http)?;
            if body_bytes.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(DriverError::BodyLimit {
                    limit: self.max_response_bytes,
                });
            }
            body_bytes.extend_from_slice(&chunk);
        }
        let body = String::from_utf8_lossy(&body_bytes).into_owned();

        if !status.is_success() {
            // SOAP Faults may still arrive with HTTP 500; surface both.
            if let Ok(fault) = soap::parse_fault(&body) {
                return Err(DriverError::http(format!(
                    "SOAP Fault: code={}, reason={}",
                    fault.code, fault.reason
                )));
            }
            return Err(DriverError::HttpStatus {
                status: status.as_u16(),
                body: body.chars().take(512).collect(),
            });
        }

        // HTTP 200 with SOAP Fault body.
        if body.contains("Fault")
            && let Ok(fault) = soap::parse_fault(&body)
        {
            return Err(DriverError::http(format!(
                "SOAP Fault: code={}, reason={}",
                fault.code, fault.reason
            )));
        }

        let _ = StatusCode::from_u16(status.as_u16());
        Ok(body)
    }

    /// Returns true when all request permits are currently in use.
    pub fn is_request_queue_saturated(&self) -> bool {
        self.permits.available_permits() == 0
    }

    /// Returns the default HTTP request timeout.
    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }
}
