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
use std::time::Duration;
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
        })
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
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_device_information_request(&msg_id)?;
        let action = "http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation";
        let body = self
            .post_with_optional_auth(endpoint, action, &req, credentials, timeout)
            .await?;
        Ok(parse_get_device_information_response(&body, &self.limits)?)
    }

    /// Fetches system date and time (unauthenticated).
    pub async fn get_system_date_and_time(
        &self,
        endpoint: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<cheetah_onvif_module::services::SystemDateAndTime> {
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_system_date_and_time_request(&msg_id)?;
        let body = self
            .client
            .post(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetSystemDateAndTime",
                &req,
                timeout,
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

        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_services_request(include_capabilities, &msg_id)?;
        match self
            .post_with_optional_auth(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetServices",
                &req,
                credentials,
                timeout,
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

        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_capabilities_request(&msg_id)?;
        match self
            .post_with_optional_auth(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetCapabilities",
                &req,
                credentials,
                timeout,
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
        let order = match prefer {
            MediaDialect::Media2 => [MediaDialect::Media2, MediaDialect::Media1],
            MediaDialect::Media1 => [MediaDialect::Media1, MediaDialect::Media2],
        };
        let mut last_err = None;
        for dialect in order {
            match self
                .get_profiles_dialect(media_endpoint, dialect, credentials, timeout)
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
            .post_with_optional_auth(media_endpoint, action, &req, credentials, timeout)
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
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_snapshot_uri_request(dialect, profile_token, &msg_id)?;
        let action = match dialect {
            MediaDialect::Media1 => "http://www.onvif.org/ver10/media/wsdl/GetSnapshotUri",
            MediaDialect::Media2 => "http://www.onvif.org/ver20/media/wsdl/GetSnapshotUri",
        };
        let body = self
            .post_with_optional_auth(media_endpoint, action, &req, credentials, timeout)
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
