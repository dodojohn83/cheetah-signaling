//! Tokio driver for ONVIF: WS-Discovery over UDP and SOAP 1.2 over HTTP.
//!
//! Business mapping lives in `cheetah-onvif-module`. This crate only performs
//! network I/O with deadlines, body limits and SSRF policy enforcement.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod auth;
pub mod capability_cache;
pub mod config;
pub mod discovery;
pub mod error;
pub mod protocol_driver;
pub mod soap_client;

pub use auth::{DeviceCredentials, inject_username_token};
pub use config::DriverConfig;
pub use discovery::{DiscoveryResult, probe_once, validate_endpoint};
pub use error::{DriverError, DriverResult};
pub use protocol_driver::{OnvifTokioDriverFactory, OnvifTokioProtocolDriver};
pub use soap_client::SoapClient;

use crate::capability_cache::CapabilityCache;
use cheetah_onvif_module::services::{
    MediaDialect, MediaProfile, SnapshotUri, StreamUri, get_capabilities_request,
    get_device_information_request, get_profiles_request, get_services_request,
    get_snapshot_uri_request, get_stream_uri_request_media1, get_stream_uri_request_media2,
    get_system_date_and_time_request, parse_get_capabilities_response,
    parse_get_device_information_response, parse_get_profiles_response,
    parse_get_services_response, parse_get_snapshot_uri_response, parse_get_stream_uri_response,
};
use cheetah_onvif_module::{
    CapabilityKind, CapabilityProbeResult, DeviceInformation, ParserLimits, Service, XAddrPolicy,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OwnedSemaphorePermit;
use tracing::warn;
use uuid::Uuid;

/// High-level helper that pairs a SOAP client with parser limits.
#[derive(Debug, Clone)]
pub struct OnvifHttpDriver {
    client: SoapClient,
    limits: ParserLimits,
    policy: XAddrPolicy,
    capability_cache: CapabilityCache,
    capability_ttl: Duration,
    per_device_concurrency: usize,
    max_tracked_device_endpoints: usize,
    device_permits: Arc<Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>>,
}

impl OnvifHttpDriver {
    /// Creates a driver from configuration.
    pub fn new(config: &DriverConfig) -> DriverResult<Self> {
        Ok(Self {
            client: SoapClient::new(config)?,
            limits: ParserLimits::default(),
            policy: config.xaddr_policy.clone(),
            capability_cache: CapabilityCache::new(config.capability_cache_capacity),
            capability_ttl: config.capability_ttl,
            per_device_concurrency: config.per_device_concurrency.max(1),
            max_tracked_device_endpoints: config.max_tracked_device_endpoints.max(1),
            device_permits: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Acquires a per-endpoint concurrency permit, waiting at most `timeout`.
    ///
    /// This limits the number of in-flight ONVIF service calls to the same
    /// device so a single camera is not overwhelmed by concurrent workflows.
    /// Idle semaphores are evicted when the number of tracked endpoints grows
    /// beyond the configured limit, but only when no caller holds or waits for a
    /// permit from that endpoint.
    async fn acquire_device_permit(
        &self,
        endpoint: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<OwnedSemaphorePermit> {
        let semaphore = {
            let mut guard = self
                .device_permits
                .lock()
                .map_err(|_| DriverError::Config("device permit map poisoned".into()))?;

            while guard.len() >= self.max_tracked_device_endpoints {
                let evictable: Vec<String> = guard
                    .iter()
                    .filter(|(_, sem)| {
                        Arc::strong_count(sem) == 1
                            && sem.available_permits() == self.per_device_concurrency
                    })
                    .map(|(k, _)| k.clone())
                    .collect();
                if evictable.is_empty() {
                    break;
                }
                for key in evictable {
                    guard.remove(&key);
                    if guard.len() < self.max_tracked_device_endpoints {
                        break;
                    }
                }
            }

            guard
                .entry(endpoint.to_string())
                .or_insert_with(|| {
                    Arc::new(tokio::sync::Semaphore::new(self.per_device_concurrency))
                })
                .clone()
        };
        let timeout = timeout.unwrap_or_else(|| self.client.request_timeout());
        let permit = tokio::time::timeout(timeout, semaphore.acquire_owned())
            .await
            .map_err(|_| {
                DriverError::Timeout(format!("timed out acquiring device permit for {endpoint}"))
            })?
            .map_err(|_| DriverError::Config("device permit semaphore closed".into()))?;
        Ok(permit)
    }

    /// Fetches device information.
    ///
    /// When `credentials` are supplied the request is signed with a WS-Security
    /// UsernameToken; otherwise the device is queried unauthenticated.
    pub async fn get_device_information(
        &self,
        endpoint: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<DeviceInformation> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = self
            .acquire_device_permit(endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_device_information_request(&msg_id)?;
        let action = "http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation";
        let body = self
            .post_with_optional_auth(endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(parse_get_device_information_response(&body, &self.limits)?)
    }

    /// Fetches system date and time (unauthenticated).
    pub async fn get_system_date_and_time(
        &self,
        endpoint: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<cheetah_onvif_module::services::SystemDateAndTime> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = self
            .acquire_device_permit(endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_system_date_and_time_request(&msg_id)?;
        let body = self
            .client
            .post(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetSystemDateAndTime",
                &req,
                http_timeout,
            )
            .await?;
        Ok(cheetah_onvif_module::services::parse_get_system_date_and_time_response(&body)?)
    }

    /// Fetches the ONVIF service list, using a per-tenant/endpoint cache and TTL.
    ///
    /// If the cache has a non-expired entry for the same `tenant_id` and
    /// `credentials` it is returned directly. When a refresh fails, the previous
    /// entry is still returned so callers do not lose the last known service list.
    pub async fn get_services(
        &self,
        endpoint: &str,
        tenant_id: Option<&str>,
        include_capabilities: bool,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<Vec<Service>> {
        let key = cache_key(endpoint, tenant_id, credentials);

        if !self.capability_ttl.is_zero()
            && let Some(services) = self
                .capability_cache
                .get_services(&key, self.capability_ttl)
        {
            return Ok(services);
        }

        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = match self
            .acquire_device_permit(endpoint, resolve_timeout(deadline)?)
            .await
        {
            Ok(permit) => permit,
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_services(&key) {
                    warn!("returning stale ONVIF services after permit timeout");
                    return Ok(stale);
                }
                return Err(e);
            }
        };
        let http_timeout = match resolve_timeout(deadline) {
            Ok(t) => t,
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_services(&key) {
                    warn!("returning stale ONVIF services after deadline exceeded");
                    return Ok(stale);
                }
                return Err(e);
            }
        };
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_services_request(include_capabilities, &msg_id)?;
        match self
            .post_with_optional_auth(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetServices",
                &req,
                credentials,
                http_timeout,
            )
            .await
        {
            Ok(body) => match parse_get_services_response(&body, &self.limits, &self.policy) {
                Ok(services) => {
                    if !self.capability_ttl.is_zero() {
                        self.capability_cache.set_services(
                            &key,
                            services.clone(),
                            self.capability_ttl,
                        );
                    }
                    Ok(services)
                }
                Err(e) => {
                    if let Some(stale) = self.capability_cache.stale_services(&key) {
                        warn!("returning stale ONVIF services after parse failure");
                        return Ok(stale);
                    }
                    Err(e.into())
                }
            },
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_services(&key) {
                    warn!("returning stale ONVIF services after refresh failure");
                    return Ok(stale);
                }
                Err(e)
            }
        }
    }

    /// Fetches the ONVIF capabilities map, using a per-tenant/endpoint cache and TTL.
    ///
    /// If the cache has a non-expired entry for the same `tenant_id` and
    /// `credentials` it is returned directly. When a refresh fails, the previous
    /// entry is still returned so callers do not lose the last known capabilities.
    pub async fn get_capabilities(
        &self,
        endpoint: &str,
        tenant_id: Option<&str>,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<HashMap<CapabilityKind, CapabilityProbeResult>> {
        let key = cache_key(endpoint, tenant_id, credentials);

        if !self.capability_ttl.is_zero()
            && let Some(caps) = self
                .capability_cache
                .get_capabilities(&key, self.capability_ttl)
        {
            return Ok(caps);
        }

        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = match self
            .acquire_device_permit(endpoint, resolve_timeout(deadline)?)
            .await
        {
            Ok(permit) => permit,
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_capabilities(&key) {
                    warn!("returning stale ONVIF capabilities after permit timeout");
                    return Ok(stale);
                }
                return Err(e);
            }
        };
        let http_timeout = match resolve_timeout(deadline) {
            Ok(t) => t,
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_capabilities(&key) {
                    warn!("returning stale ONVIF capabilities after deadline exceeded");
                    return Ok(stale);
                }
                return Err(e);
            }
        };
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_capabilities_request(&msg_id)?;
        match self
            .post_with_optional_auth(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetCapabilities",
                &req,
                credentials,
                http_timeout,
            )
            .await
        {
            Ok(body) => match parse_get_capabilities_response(&body, &self.limits) {
                Ok(caps) => {
                    if !self.capability_ttl.is_zero() {
                        self.capability_cache.set_capabilities(
                            &key,
                            caps.clone(),
                            self.capability_ttl,
                        );
                    }
                    Ok(caps)
                }
                Err(e) => {
                    if let Some(stale) = self.capability_cache.stale_capabilities(&key) {
                        warn!("returning stale ONVIF capabilities after parse failure");
                        return Ok(stale);
                    }
                    Err(e.into())
                }
            },
            Err(e) => {
                if let Some(stale) = self.capability_cache.stale_capabilities(&key) {
                    warn!("returning stale ONVIF capabilities after refresh failure");
                    return Ok(stale);
                }
                Err(e)
            }
        }
    }

    /// Lists media profiles, preferring Media2 then falling back to Media1.
    ///
    /// When `credentials` are supplied the request is signed with a WS-Security
    /// UsernameToken; otherwise the device is queried unauthenticated.
    pub async fn get_profiles(
        &self,
        media_endpoint: &str,
        prefer: MediaDialect,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<(MediaDialect, Vec<MediaProfile>)> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = self
            .acquire_device_permit(media_endpoint, resolve_timeout(deadline)?)
            .await?;
        let order = match prefer {
            MediaDialect::Media2 => [MediaDialect::Media2, MediaDialect::Media1],
            MediaDialect::Media1 => [MediaDialect::Media1, MediaDialect::Media2],
        };
        let mut last_err = None;
        for dialect in order {
            match self
                .get_profiles_dialect(
                    media_endpoint,
                    dialect,
                    credentials,
                    resolve_timeout(deadline)?,
                )
                .await
            {
                Ok(profiles) if !profiles.is_empty() => return Ok((dialect, profiles)),
                Ok(profiles) => return Ok((dialect, profiles)),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| DriverError::Config("no media dialect succeeded".into())))
    }

    async fn get_profiles_dialect(
        &self,
        media_endpoint: &str,
        dialect: MediaDialect,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<Vec<MediaProfile>> {
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_profiles_request(dialect, &msg_id)?;
        let action = match dialect {
            MediaDialect::Media1 => "http://www.onvif.org/ver10/media/wsdl/GetProfiles",
            MediaDialect::Media2 => "http://www.onvif.org/ver20/media/wsdl/GetProfiles",
        };
        let body = self
            .post_with_optional_auth(media_endpoint, action, &req, credentials, timeout)
            .await?;
        Ok(parse_get_profiles_response(&body, &self.limits)?)
    }

    /// Fetches a stream URI for a profile.
    ///
    /// When `credentials` are supplied the request is signed with a WS-Security
    /// UsernameToken; otherwise the device is queried unauthenticated.
    pub async fn get_stream_uri(
        &self,
        media_endpoint: &str,
        dialect: MediaDialect,
        profile_token: &str,
        protocol: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<StreamUri> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = self
            .acquire_device_permit(media_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let (action, req) = match dialect {
            MediaDialect::Media1 => (
                "http://www.onvif.org/ver10/media/wsdl/GetStreamUri",
                get_stream_uri_request_media1(profile_token, "RTP-Unicast", protocol, &msg_id)?,
            ),
            MediaDialect::Media2 => (
                "http://www.onvif.org/ver20/media/wsdl/GetStreamUri",
                get_stream_uri_request_media2(profile_token, protocol, &msg_id)?,
            ),
        };
        let body = self
            .post_with_optional_auth(media_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(parse_get_stream_uri_response(
            &body,
            &self.limits,
            &self.policy,
        )?)
    }

    /// Fetches a snapshot URI for a profile.
    ///
    /// When `credentials` are supplied the request is signed with a WS-Security
    /// UsernameToken; otherwise the device is queried unauthenticated.
    pub async fn get_snapshot_uri(
        &self,
        media_endpoint: &str,
        dialect: MediaDialect,
        profile_token: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<SnapshotUri> {
        let deadline = timeout.map(|d| Instant::now() + d);
        let _permit = self
            .acquire_device_permit(media_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_snapshot_uri_request(dialect, profile_token, &msg_id)?;
        let action = match dialect {
            MediaDialect::Media1 => "http://www.onvif.org/ver10/media/wsdl/GetSnapshotUri",
            MediaDialect::Media2 => "http://www.onvif.org/ver20/media/wsdl/GetSnapshotUri",
        };
        let body = self
            .post_with_optional_auth(media_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(parse_get_snapshot_uri_response(
            &body,
            &self.limits,
            &self.policy,
        )?)
    }

    async fn post_with_optional_auth(
        &self,
        endpoint: &str,
        action: &str,
        envelope: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<String> {
        match credentials {
            Some(creds) => {
                self.client
                    .post_authenticated(endpoint, action, envelope, creds, timeout)
                    .await
            }
            None => self.client.post(endpoint, action, envelope, timeout).await,
        }
    }

    /// Returns true when the underlying SOAP client request queue is saturated.
    pub fn is_request_queue_saturated(&self) -> bool {
        self.client.is_request_queue_saturated()
    }
}

/// Computes the time remaining until `deadline`, or `None` if there is no
/// deadline. Returns `DriverError::Timeout` when the deadline has already
/// passed, so the same caller-facing timeout is shared between permit
/// acquisition and the subsequent HTTP request.
fn resolve_timeout(deadline: Option<Instant>) -> DriverResult<Option<Duration>> {
    match deadline {
        None => Ok(None),
        Some(deadline) => {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                Err(DriverError::Timeout("deadline exceeded".into()))
            } else {
                Ok(Some(remaining))
            }
        }
    }
}

fn cache_key(
    endpoint: &str,
    tenant_id: Option<&str>,
    credentials: Option<&DeviceCredentials>,
) -> String {
    // The cache is scoped by tenant and a stable credential digest so callers with
    // different tenants or credentials (including the same username but a
    // different password) cannot reuse each other's cached results.
    let credential_id = credentials
        .map(|c| {
            use secrecy::ExposeSecret;
            use sha2::{Digest, Sha256};
            let password = c.password.expose_secret();
            let mut hasher = Sha256::new();
            hasher.update(password.as_bytes());
            let digest = hex::encode(hasher.finalize());
            format!("{}:{digest}", c.username)
        })
        .unwrap_or_else(|| "anonymous".to_string());
    let tenant = tenant_id.unwrap_or("default");
    format!("{tenant}#{credential_id}#{endpoint}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn per_device_concurrency_limits_concurrent_calls() {
        let config = DriverConfig {
            per_device_concurrency: 1,
            ..Default::default()
        };
        let driver = OnvifHttpDriver::new(&config).expect("driver should build");
        let endpoint = "http://example.com/onvif/device_service";

        let _first = driver
            .acquire_device_permit(endpoint, None)
            .await
            .expect("first permit should be available");

        let result = driver
            .acquire_device_permit(endpoint, Some(Duration::from_nanos(0)))
            .await;
        assert!(
            matches!(result, Err(DriverError::Timeout(_))),
            "second caller should be denied while the first permit is held, got {result:?}"
        );
    }

    #[tokio::test]
    async fn idle_device_permits_are_evicted_when_map_exceeds_capacity() {
        let config = DriverConfig {
            per_device_concurrency: 1,
            max_tracked_device_endpoints: 1,
            ..Default::default()
        };
        let driver = OnvifHttpDriver::new(&config).expect("driver should build");
        let first_endpoint = "http://a.onvif/device_service";
        let second_endpoint = "http://b.onvif/device_service";

        let first = driver
            .acquire_device_permit(first_endpoint, None)
            .await
            .expect("first permit should be available");
        drop(first);

        let _second = driver
            .acquire_device_permit(second_endpoint, None)
            .await
            .expect("second permit should be available");

        let tracked = driver.device_permits.lock().expect("lock map").len();
        assert_eq!(
            tracked, 1,
            "idle first entry should be evicted to keep map bounded"
        );
    }
}
