use crate::capability_cache::CapabilityCache;
use crate::util::deadline_from_now;
use crate::{
    DeviceCredentials, DriverConfig, DriverError, DriverResult, SoapClient, validate_endpoint,
};
use cheetah_onvif_services::services::{
    MediaDialect, MediaProfile, OnvifNotification, PtzPreset, PtzVelocity, PullPointSubscription,
    RENEW_ACTION, SnapshotUri, StreamUri, UNSUBSCRIBE_ACTION, continuous_move_request,
    create_pull_point_subscription_request, get_capabilities_request,
    get_device_information_request, get_presets_request, get_profiles_request,
    get_services_request, get_snapshot_uri_request, get_stream_uri_request_media1,
    get_stream_uri_request_media2, get_system_date_and_time_request,
    parse_create_pull_point_response, parse_get_capabilities_response,
    parse_get_device_information_response, parse_get_presets_response, parse_get_profiles_response,
    parse_get_services_response, parse_get_snapshot_uri_response, parse_get_stream_uri_response,
    parse_pull_messages_response, pull_messages_request, renew_request, stop_request,
    unsubscribe_request,
};
use cheetah_onvif_services::{
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
        // Reject malformed or credential-bearing endpoints before they can be
        // used as cache keys or embedded in timeout/overload error messages.
        let _ = validate_endpoint(endpoint, &self.policy)?;

        let semaphore = {
            let mut guard = self
                .device_permits
                .lock()
                .map_err(|_| DriverError::config("device permit map poisoned"))?;

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

            if guard.len() >= self.max_tracked_device_endpoints && !guard.contains_key(endpoint) {
                return Err(DriverError::overloaded(format!(
                    "max tracked device endpoints ({}) reached",
                    self.max_tracked_device_endpoints
                )));
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
                DriverError::timeout(format!("timed out acquiring device permit for {endpoint}"))
            })?
            .map_err(|_| DriverError::config("device permit semaphore closed"))?;
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
        let deadline = deadline_from_now(timeout);
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
    ) -> DriverResult<cheetah_onvif_services::services::SystemDateAndTime> {
        let deadline = deadline_from_now(timeout);
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
        Ok(cheetah_onvif_services::services::parse_get_system_date_and_time_response(&body)?)
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

        let deadline = deadline_from_now(timeout);
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

        let deadline = deadline_from_now(timeout);
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
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(media_endpoint, resolve_timeout(deadline)?)
            .await?;
        let order = match prefer {
            MediaDialect::Media2 => [MediaDialect::Media2, MediaDialect::Media1],
            MediaDialect::Media1 => [MediaDialect::Media1, MediaDialect::Media2],
        };
        let mut last_err = None;
        let mut last_empty: Option<(MediaDialect, Vec<MediaProfile>)> = None;
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
                Ok(profiles) => {
                    last_empty = Some((dialect, profiles));
                }
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(result) = last_empty {
            return Ok(result);
        }
        Err(last_err.unwrap_or_else(|| DriverError::config("no media dialect succeeded")))
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
        let deadline = deadline_from_now(timeout);
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
        let deadline = deadline_from_now(timeout);
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

    /// Lists PTZ presets for a profile.
    ///
    /// When `credentials` are supplied the request is signed with a WS-Security
    /// UsernameToken; otherwise the device is queried unauthenticated.
    pub async fn get_ptz_presets(
        &self,
        ptz_endpoint: &str,
        profile_token: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<Vec<PtzPreset>> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(ptz_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_presets_request(profile_token, &msg_id)?;
        let action = "http://www.onvif.org/ver20/ptz/wsdl/GetPresets";
        let body = self
            .post_with_optional_auth(ptz_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(parse_get_presets_response(&body, &self.limits)?)
    }

    /// Starts a continuous PTZ move with a device-side timeout.
    ///
    /// Callers must provide a `timeout_seconds` so the device will stop the move
    /// automatically; a dedicated `ptz_stop` is also available for explicit stop.
    pub async fn ptz_continuous_move(
        &self,
        ptz_endpoint: &str,
        profile_token: &str,
        velocity: PtzVelocity,
        timeout_seconds: u64,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<()> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(ptz_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = continuous_move_request(profile_token, velocity, Some(timeout_seconds), &msg_id)?;
        let action = "http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove";
        self.post_with_optional_auth(ptz_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(())
    }

    /// Stops an active PTZ move.
    ///
    /// When `pan_tilt` and `zoom` are both `true` the entire move is stopped.
    pub async fn ptz_stop(
        &self,
        ptz_endpoint: &str,
        profile_token: &str,
        pan_tilt: bool,
        zoom: bool,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<()> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(ptz_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = stop_request(profile_token, pan_tilt, zoom, &msg_id)?;
        let action = "http://www.onvif.org/ver20/ptz/wsdl/Stop";
        self.post_with_optional_auth(ptz_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(())
    }

    /// Creates a bounded pull-point subscription for ONVIF events.
    ///
    /// The returned subscription reference must be used for `pull_messages` and
    /// `renew_pull_point_subscription` calls.
    pub async fn create_pull_point_subscription(
        &self,
        events_endpoint: &str,
        initial_termination_time: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<PullPointSubscription> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(events_endpoint, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = create_pull_point_subscription_request(initial_termination_time, &msg_id)?;
        let action = "http://www.onvif.org/ver10/events/wsdl/CreatePullPointSubscription";
        let body = self
            .post_with_optional_auth(events_endpoint, action, &req, credentials, http_timeout)
            .await?;
        Ok(parse_create_pull_point_response(
            &body,
            &self.limits,
            &self.policy,
        )?)
    }

    /// Pulls a bounded number of ONVIF event notifications from a subscription.
    ///
    /// `message_limit` caps the number of notifications returned in a single call.
    /// The ONVIF `Timeout` long-poll value (`timeout_str`, e.g. `PT30S`) must be
    /// shorter than the caller-provided HTTP deadline, otherwise the request will
    /// time out at the client before the device returns an empty/partial batch.
    pub async fn pull_messages(
        &self,
        subscription_reference: &str,
        timeout_str: &str,
        message_limit: u32,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<Vec<OnvifNotification>> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(subscription_reference, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = pull_messages_request(timeout_str, message_limit, &msg_id)?;
        let action = "http://www.onvif.org/ver10/events/wsdl/PullPointSubscription/PullMessages";
        let body = self
            .post_with_optional_auth(
                subscription_reference,
                action,
                &req,
                credentials,
                http_timeout,
            )
            .await?;
        Ok(parse_pull_messages_response(
            &body,
            &self.limits,
            message_limit as usize,
        )?)
    }

    /// Renews an existing pull-point subscription with a new termination time.
    ///
    /// `subscription_reference` is the endpoint returned by `create_pull_point_subscription`.
    pub async fn renew_pull_point_subscription(
        &self,
        subscription_reference: &str,
        termination_time: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<()> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(subscription_reference, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = renew_request(termination_time, &msg_id)?;
        self.post_with_optional_auth(
            subscription_reference,
            RENEW_ACTION,
            &req,
            credentials,
            http_timeout,
        )
        .await?;
        Ok(())
    }

    /// Unsubscribes and tears down an existing pull-point subscription.
    ///
    /// `subscription_reference` is the endpoint returned by `create_pull_point_subscription`.
    /// Cancellation is bounded by `timeout` and the request is sent best-effort.
    pub async fn unsubscribe_pull_point(
        &self,
        subscription_reference: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<()> {
        let deadline = deadline_from_now(timeout);
        let _permit = self
            .acquire_device_permit(subscription_reference, resolve_timeout(deadline)?)
            .await?;
        let http_timeout = resolve_timeout(deadline)?;
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = unsubscribe_request(&msg_id)?;
        self.post_with_optional_auth(
            subscription_reference,
            UNSUBSCRIBE_ACTION,
            &req,
            credentials,
            http_timeout,
        )
        .await?;
        Ok(())
    }

    async fn post_with_optional_auth(
        &self,
        endpoint: &str,
        action: &str,
        envelope: &str,
        credentials: Option<&DeviceCredentials>,
        timeout: Option<Duration>,
    ) -> DriverResult<String> {
        // Every outbound SOAP target must pass the configured SSRF policy before
        // credentials or a request body are transmitted.
        let _ = validate_endpoint(endpoint, &self.policy)?;
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
                Err(DriverError::timeout("deadline exceeded"))
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
#[path = "driver_tests.rs"]
mod tests;
