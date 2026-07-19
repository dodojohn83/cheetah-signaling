//! Tokio driver for ONVIF: WS-Discovery over UDP and SOAP 1.2 over HTTP.
//!
//! Business mapping lives in `cheetah-onvif-module`. This crate only performs
//! network I/O with deadlines, body limits and SSRF policy enforcement.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod auth;
pub mod config;
pub mod discovery;
pub mod error;
pub mod soap_client;

pub use auth::{DeviceCredentials, inject_username_token};
pub use config::DriverConfig;
pub use discovery::{DiscoveryResult, probe_once, validate_endpoint};
pub use error::{DriverError, DriverResult};
pub use soap_client::SoapClient;

use cheetah_onvif_module::services::{
    MediaDialect, MediaProfile, SnapshotUri, StreamUri, get_device_information_request,
    get_profiles_request, get_snapshot_uri_request, get_stream_uri_request_media1,
    get_stream_uri_request_media2, get_system_date_and_time_request,
    parse_get_device_information_response, parse_get_profiles_response,
    parse_get_snapshot_uri_response, parse_get_stream_uri_response,
};
use cheetah_onvif_module::{DeviceInformation, ParserLimits, XAddrPolicy};
use std::time::Duration;
use uuid::Uuid;

/// High-level helper that pairs a SOAP client with parser limits.
#[derive(Debug, Clone)]
pub struct OnvifHttpDriver {
    client: SoapClient,
    limits: ParserLimits,
    policy: XAddrPolicy,
}

impl OnvifHttpDriver {
    /// Creates a driver from configuration.
    pub fn new(config: &DriverConfig) -> DriverResult<Self> {
        Ok(Self {
            client: SoapClient::new(config)?,
            limits: ParserLimits::default(),
            policy: config.xaddr_policy.clone(),
        })
    }

    /// Fetches device information.
    pub async fn get_device_information(
        &self,
        endpoint: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<DeviceInformation> {
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_device_information_request(&msg_id)?;
        let body = self
            .client
            .post(
                endpoint,
                "http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation",
                &req,
                timeout,
            )
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
        Ok(cheetah_onvif_module::services::parse_get_system_date_and_time_response(
            &body,
        )?)
    }

    /// Lists media profiles, preferring Media2 then falling back to Media1.
    pub async fn get_profiles(
        &self,
        media_endpoint: &str,
        prefer: MediaDialect,
        timeout: Option<Duration>,
    ) -> DriverResult<(MediaDialect, Vec<MediaProfile>)> {
        let order = match prefer {
            MediaDialect::Media2 => [MediaDialect::Media2, MediaDialect::Media1],
            MediaDialect::Media1 => [MediaDialect::Media1, MediaDialect::Media2],
        };
        let mut last_err = None;
        for dialect in order {
            match self.get_profiles_dialect(media_endpoint, dialect, timeout).await {
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
        timeout: Option<Duration>,
    ) -> DriverResult<Vec<MediaProfile>> {
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_profiles_request(dialect, &msg_id)?;
        let action = match dialect {
            MediaDialect::Media1 => "http://www.onvif.org/ver10/media/wsdl/GetProfiles",
            MediaDialect::Media2 => "http://www.onvif.org/ver20/media/wsdl/GetProfiles",
        };
        let body = self
            .client
            .post(media_endpoint, action, &req, timeout)
            .await?;
        Ok(parse_get_profiles_response(&body, &self.limits)?)
    }

    /// Fetches a stream URI for a profile.
    pub async fn get_stream_uri(
        &self,
        media_endpoint: &str,
        dialect: MediaDialect,
        profile_token: &str,
        protocol: &str,
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
            .client
            .post(media_endpoint, action, &req, timeout)
            .await?;
        Ok(parse_get_stream_uri_response(
            &body,
            &self.limits,
            &self.policy,
        )?)
    }

    /// Fetches a snapshot URI for a profile.
    pub async fn get_snapshot_uri(
        &self,
        media_endpoint: &str,
        dialect: MediaDialect,
        profile_token: &str,
        timeout: Option<Duration>,
    ) -> DriverResult<SnapshotUri> {
        let msg_id = format!("urn:uuid:{}", Uuid::now_v7());
        let req = get_snapshot_uri_request(dialect, profile_token, &msg_id)?;
        let action = match dialect {
            MediaDialect::Media1 => "http://www.onvif.org/ver10/media/wsdl/GetSnapshotUri",
            MediaDialect::Media2 => "http://www.onvif.org/ver20/media/wsdl/GetSnapshotUri",
        };
        let body = self
            .client
            .post(media_endpoint, action, &req, timeout)
            .await?;
        Ok(parse_get_snapshot_uri_response(
            &body,
            &self.limits,
            &self.policy,
        )?)
    }
}
