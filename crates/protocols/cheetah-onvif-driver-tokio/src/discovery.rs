//! WS-Discovery Probe over UDP multicast.

use crate::config::DriverConfig;
use crate::error::{DriverError, DriverResult};
use crate::util::{clamp_timeout, deadline_from_now};
use cheetah_onvif_core::discovery::{
    AppId, DiscoveryLimits, ProbeMatch, XAddrPolicy, build_probe, check_datagram_size,
    filter_xaddrs, parse_probe_matches_with_limits,
};
use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use uuid::Uuid;

/// Maximum byte length of an ONVIF endpoint URL that the driver will parse.
///
/// ONVIF XAddr URLs are short service addresses; capping them prevents a
/// misbehaving command payload from forcing `url::Url::parse` and the permit
/// cache to allocate multi-kilobyte or multi-megabyte keys.
const MAX_ENDPOINT_BYTES: usize = 4096;

/// Result of a discovery probe round.
#[derive(Clone, Debug, Default)]
pub struct DiscoveryResult {
    /// Deduplicated probe matches with filtered XAddrs.
    pub matches: Vec<ProbeMatch>,
    /// Number of datagrams received.
    pub datagrams_received: u32,
    /// Number of datagrams rejected by limits/policy.
    pub datagrams_rejected: u32,
}

/// Runs a single WS-Discovery Probe and collects ProbeMatches until timeout.
pub async fn probe_once(config: &DriverConfig) -> DriverResult<DiscoveryResult> {
    let socket = UdpSocket::bind(config.discovery_bind)
        .await
        .map_err(DriverError::Io)?;
    socket.set_broadcast(true).map_err(DriverError::Io)?;

    let app_id = AppId::new(format!("urn:uuid:{}", Uuid::now_v7()));
    let types = vec!["dp0:NetworkVideoTransmitter".to_string()];
    let probe_xml = build_probe(&app_id, &types, None)?;
    check_datagram_size(&probe_xml, &config.discovery_limits).map_err(DriverError::Onvif)?;

    socket
        .send_to(probe_xml.as_bytes(), config.discovery_multicast)
        .await
        .map_err(DriverError::Io)?;

    let timeout = clamp_timeout(Duration::from_millis(
        config.discovery_timeout.as_millis().max(0) as u64,
    ));
    collect_matches(
        &socket,
        &config.discovery_limits,
        &config.xaddr_policy,
        timeout,
    )
    .await
}

async fn collect_matches(
    socket: &UdpSocket,
    limits: &DiscoveryLimits,
    policy: &XAddrPolicy,
    timeout: Duration,
) -> DriverResult<DiscoveryResult> {
    let deadline = deadline_from_now(Some(timeout));
    let max_datagram_bytes = limits.max_datagram_bytes.max(512);
    let mut buf = vec![0u8; max_datagram_bytes];
    let mut result = DiscoveryResult::default();
    let mut seen: HashSet<String> = HashSet::new();

    let deadline = deadline.unwrap_or_else(Instant::now);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let recv = tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await;
        match recv {
            Ok(Ok((len, _from))) => {
                result.datagrams_received += 1;
                let data = &buf[..len];
                let text = match std::str::from_utf8(data) {
                    Ok(t) => t,
                    Err(_) => {
                        result.datagrams_rejected += 1;
                        continue;
                    }
                };
                if check_datagram_size(text, limits).is_err() {
                    result.datagrams_rejected += 1;
                    continue;
                }
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let matches = match parse_probe_matches_with_limits(text, now, limits) {
                    Ok(m) => m,
                    Err(_) => {
                        result.datagrams_rejected += 1;
                        continue;
                    }
                };
                for mut m in matches.matches {
                    let key = m.endpoint_reference.0.clone();
                    if !seen.insert(key) {
                        continue;
                    }
                    let filtered = filter_xaddrs(&m.x_addrs.0, policy.allow_private);
                    let mut kept = Vec::new();
                    for x in filtered {
                        if let Ok(url) = url::Url::parse(&x)
                            && policy.validate(&url).is_ok()
                        {
                            kept.push(x);
                        }
                    }
                    if kept.is_empty() {
                        result.datagrams_rejected += 1;
                        continue;
                    }
                    m.x_addrs.0 = kept;
                    result.matches.push(m);
                    if result.matches.len() >= limits.max_matches {
                        return Ok(result);
                    }
                }
            }
            Ok(Err(e)) => return Err(DriverError::Io(e)),
            Err(_) => break, // elapsed
        }
    }

    Ok(result)
}

/// Validates a configured device endpoint against SSRF policy before HTTP.
///
/// Rejects URLs that embed credentials in the userinfo section; ONVIF devices
/// are authenticated via WS-Security UsernameToken, so any userinfo in the
/// XAddr is a potential secret leak and should not be transmitted.
pub fn validate_endpoint(endpoint: &str, policy: &XAddrPolicy) -> DriverResult<url::Url> {
    if endpoint.is_empty() {
        return Err(DriverError::Onvif(
            cheetah_onvif_core::OnvifError::invalid_xaddr("endpoint is empty"),
        ));
    }
    if endpoint.len() > MAX_ENDPOINT_BYTES {
        return Err(DriverError::Onvif(
            cheetah_onvif_core::OnvifError::invalid_xaddr("endpoint exceeds maximum length"),
        ));
    }

    let mut url = url::Url::parse(endpoint)
        .map_err(|e| DriverError::Onvif(cheetah_onvif_core::OnvifError::invalid_xaddr(e)))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DriverError::Onvif(
            cheetah_onvif_core::OnvifError::invalid_xaddr("endpoint must not contain userinfo"),
        ));
    }
    policy.validate(&url).map_err(DriverError::Onvif)?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn test_policy() -> XAddrPolicy {
        XAddrPolicy {
            allow_private: true,
            ..XAddrPolicy::default()
        }
    }

    #[test]
    fn validate_endpoint_accepts_valid_url() {
        let url = validate_endpoint("http://192.168.1.1/onvif/device_service", &test_policy())
            .expect("valid endpoint should pass");
        assert_eq!(url.host_str(), Some("192.168.1.1"));
    }

    #[test]
    fn validate_endpoint_rejects_empty() {
        assert!(validate_endpoint("", &test_policy()).is_err());
    }

    #[test]
    fn validate_endpoint_rejects_oversized() {
        let long = "x".repeat(MAX_ENDPOINT_BYTES + 1);
        let endpoint = format!("http://192.168.1.1/{long}");
        assert!(validate_endpoint(&endpoint, &test_policy()).is_err());
    }

    #[test]
    fn validate_endpoint_rejects_userinfo() {
        assert!(validate_endpoint("http://user:pass@192.168.1.1/onvif", &test_policy()).is_err());
    }
}
