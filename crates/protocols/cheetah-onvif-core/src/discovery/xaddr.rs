//! SSRF policy and XAddr validation for ONVIF discovery results.
//!
//! The actual HTTP redirect handling belongs in the driver; this module only
//! provides policy helpers that the driver can call before fetching a URL.

use crate::error::{OnvifError, OnvifResult};
use url::Url;

/// SSRF policy for discovered transport addresses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XAddrPolicy {
    /// Allowed URL schemes (usually `http` and `https`).
    pub allowed_schemes: Vec<String>,
    /// Allowed destination ports. An empty list allows any port.
    pub allowed_ports: Vec<u16>,
    /// Whether private (RFC 1918) addresses are allowed.
    pub allow_private: bool,
    /// Whether loopback addresses are allowed.
    pub allow_loopback: bool,
    /// Whether IPv4 link-local and IPv6 unicast link-local addresses are allowed.
    pub allow_link_local: bool,
    /// Whether `0.0.0.0` / `::` is allowed.
    pub allow_unspecified: bool,
}

impl Default for XAddrPolicy {
    fn default() -> Self {
        Self {
            allowed_schemes: vec!["http".to_string(), "https".to_string()],
            allowed_ports: vec![80, 443],
            allow_private: false,
            allow_loopback: false,
            allow_link_local: false,
            allow_unspecified: false,
        }
    }
}

impl XAddrPolicy {
    /// Allows private addresses (still rejects loopback unless also enabled).
    pub fn with_allow_private(mut self, allow: bool) -> Self {
        self.allow_private = allow;
        self
    }

    /// Validates a single URL against this policy.
    pub fn validate(&self, url: &Url) -> OnvifResult<()> {
        if !self.allowed_schemes.iter().any(|s| s == url.scheme()) {
            return Err(OnvifError::SsrfRejected(format!(
                "scheme {} not allowed",
                url.scheme()
            )));
        }

        if let Some(port) = url.port()
            && !self.allowed_ports.is_empty()
            && !self.allowed_ports.contains(&port)
        {
            return Err(OnvifError::SsrfRejected(format!(
                "port {} not allowed",
                port
            )));
        }

        if !url.username().is_empty() || url.password().is_some() {
            return Err(OnvifError::SsrfRejected("userinfo not allowed".to_string()));
        }

        if let Some(segments) = url.path_segments() {
            for segment in segments {
                if segment.is_empty() {
                    return Err(OnvifError::SsrfRejected(
                        "empty path segment not allowed".to_string(),
                    ));
                }
            }
        }

        let host = url
            .host_str()
            .ok_or_else(|| OnvifError::InvalidXAddr(url.to_string()))?;

        if host == "localhost" && !self.allow_loopback {
            return Err(OnvifError::SsrfRejected(
                "localhost not allowed".to_string(),
            ));
        }

        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if !self.allow_unspecified && ip.is_unspecified() {
                return Err(OnvifError::SsrfRejected(
                    "unspecified address not allowed".to_string(),
                ));
            }
            if !self.allow_loopback && ip.is_loopback() {
                return Err(OnvifError::SsrfRejected(
                    "loopback address not allowed".to_string(),
                ));
            }
            if !self.allow_link_local && is_link_local(ip) {
                return Err(OnvifError::SsrfRejected(
                    "link-local address not allowed".to_string(),
                ));
            }
            if !self.allow_private && is_private(ip) {
                return Err(OnvifError::SsrfRejected(
                    "private address not allowed".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Filters a list of XAddrs, returning only those that pass this policy.
    pub fn filter(&self, xaddrs: &[String]) -> OnvifResult<Vec<String>> {
        let mut out = Vec::with_capacity(xaddrs.len());
        for addr in xaddrs {
            let url = Url::parse(addr).map_err(OnvifError::from)?;
            self.validate(&url)?;
            out.push(addr.clone());
        }
        Ok(out)
    }

    /// Validates a redirect target relative to the original request URL.
    ///
    /// The target must pass the same policy; in addition this helper rejects
    /// downgrades from `https` to `http` and redirects to a different authority.
    pub fn validate_redirect(&self, original: &Url, target: &Url) -> OnvifResult<()> {
        self.validate(target)?;

        if original.scheme() == "https" && target.scheme() == "http" {
            return Err(OnvifError::SsrfRejected(
                "redirect from https to http not allowed".to_string(),
            ));
        }

        if original.host_str() != target.host_str() || original.port() != target.port() {
            return Err(OnvifError::SsrfRejected(
                "redirect to different authority not allowed".to_string(),
            ));
        }

        Ok(())
    }
}

fn is_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local(),
        std::net::IpAddr::V6(v6) => v6.is_unicast_link_local(),
    }
}

fn is_private(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_private(),
        std::net::IpAddr::V6(_) => false,
    }
}

/// Convenience filter that uses a policy with the default allowed ports and
/// the supplied private-address setting. When `allow_private` is true,
/// loopback, link-local and unspecified addresses are also allowed so that
/// the flag matches the broad meaning used in earlier code.
pub fn filter_xaddrs(xaddrs: &[String], allow_private: bool) -> OnvifResult<Vec<String>> {
    let policy = XAddrPolicy {
        allow_private,
        allow_loopback: allow_private,
        allow_link_local: allow_private,
        allow_unspecified: allow_private,
        ..Default::default()
    };
    policy.filter(xaddrs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn rejects_loopback() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://127.0.0.1/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_private() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://192.168.1.1/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn allows_private_when_enabled() {
        let policy = XAddrPolicy::default().with_allow_private(true);
        assert!(
            policy
                .validate(&Url::parse("http://192.168.1.1/onvif").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn rejects_non_http_scheme() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("ftp://192.168.1.1/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_disallowed_port() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://192.168.1.1:8080/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_userinfo() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://user:pass@example.com/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_empty_path_segment() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://example.com//admin").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_https_to_http_redirect() {
        let policy = XAddrPolicy::default();
        let original = Url::parse("https://example.com/onvif").unwrap();
        let target = Url::parse("http://example.com/onvif").unwrap();
        assert!(policy.validate_redirect(&original, &target).is_err());
    }

    #[test]
    fn rejects_redirect_to_different_authority() {
        let policy = XAddrPolicy::default();
        let original = Url::parse("http://example.com/onvif").unwrap();
        let target = Url::parse("http://attacker.com/onvif").unwrap();
        assert!(policy.validate_redirect(&original, &target).is_err());
    }
}
