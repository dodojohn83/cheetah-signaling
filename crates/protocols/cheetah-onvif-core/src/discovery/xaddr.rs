//! SSRF policy and XAddr validation for ONVIF discovery results.
//!
//! The actual HTTP redirect handling belongs in the driver; this module only
//! provides policy helpers that the driver can call before fetching a URL.

use crate::error::{OnvifError, OnvifResult};
use url::{Host, Url};

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
    /// Whether domain-name hosts are allowed. Defaults to `false` because the
    /// Sans-I/O core cannot resolve or re-check DNS results; the driver should
    /// set this to `true` only after validating the resolved address against
    /// the same IP-classification policy and DNS-rebinding rules.
    pub allow_domain_names: bool,
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
            allow_domain_names: false,
        }
    }
}

impl XAddrPolicy {
    /// Allows private addresses (still rejects loopback unless also enabled).
    pub fn with_allow_private(mut self, allow: bool) -> Self {
        self.allow_private = allow;
        self
    }

    /// Allows domain-name hosts. The caller (driver) is responsible for DNS
    /// resolution and DNS-rebinding validation before fetching the URL.
    pub fn with_allow_domain_names(mut self, allow: bool) -> Self {
        self.allow_domain_names = allow;
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

        // Reject internal/leading double slashes (e.g. `/onvif//service`) while
        // still allowing root paths (`/`) and trailing slashes (`/onvif/`).
        if url.path().contains("//") {
            return Err(OnvifError::SsrfRejected(
                "empty path segment not allowed".to_string(),
            ));
        }

        let host = url
            .host()
            .ok_or_else(|| OnvifError::InvalidXAddr(url.to_string()))?;

        match host {
            Host::Domain(domain) => {
                if domain == "localhost" {
                    if !self.allow_loopback {
                        return Err(OnvifError::SsrfRejected(
                            "localhost not allowed".to_string(),
                        ));
                    }
                    return Ok(());
                }
                if let Ok(ip) = domain.parse::<std::net::IpAddr>() {
                    return self.check_ip(ip);
                }
                if !self.allow_domain_names {
                    return Err(OnvifError::SsrfRejected(
                        "domain name hosts not allowed".to_string(),
                    ));
                }
            }
            Host::Ipv4(v4) => self.check_ip(std::net::IpAddr::V4(v4))?,
            Host::Ipv6(v6) => self.check_ip(std::net::IpAddr::V6(v6))?,
        }

        Ok(())
    }

    fn check_ip(&self, ip: std::net::IpAddr) -> OnvifResult<()> {
        let (loopback, link_local, private, unspecified) = classify(ip);
        if !self.allow_unspecified && unspecified {
            return Err(OnvifError::SsrfRejected(
                "unspecified address not allowed".to_string(),
            ));
        }
        if !self.allow_loopback && loopback {
            return Err(OnvifError::SsrfRejected(
                "loopback address not allowed".to_string(),
            ));
        }
        if !self.allow_link_local && link_local {
            return Err(OnvifError::SsrfRejected(
                "link-local address not allowed".to_string(),
            ));
        }
        if !self.allow_private && private {
            return Err(OnvifError::SsrfRejected(
                "private address not allowed".to_string(),
            ));
        }
        Ok(())
    }

    /// Filters a list of XAddrs, returning only those that pass this policy.
    ///
    /// Malformed or rejected addresses are skipped rather than failing the
    /// entire list, so a device that advertises a mix of usable and blocked
    /// XAddrs is not discarded outright.
    pub fn filter(&self, xaddrs: &[String]) -> Vec<String> {
        let mut out = Vec::with_capacity(xaddrs.len());
        for addr in xaddrs {
            if let Ok(url) = Url::parse(addr)
                && self.validate(&url).is_ok()
            {
                out.push(addr.clone());
            }
        }
        out
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

        if original.host() != target.host()
            || original.port_or_known_default() != target.port_or_known_default()
        {
            return Err(OnvifError::SsrfRejected(
                "redirect to different authority not allowed".to_string(),
            ));
        }

        Ok(())
    }
}

fn classify(ip: std::net::IpAddr) -> (bool, bool, bool, bool) {
    match ip {
        std::net::IpAddr::V4(v4) => (
            v4.is_loopback(),
            v4.is_link_local(),
            v4.is_private(),
            v4.is_unspecified(),
        ),
        std::net::IpAddr::V6(v6) => {
            let mapped = v6.to_ipv4_mapped();
            (
                v6.is_loopback() || mapped.is_some_and(|v4| v4.is_loopback()),
                v6.is_unicast_link_local() || mapped.is_some_and(|v4| v4.is_link_local()),
                v6.is_unique_local() || mapped.is_some_and(|v4| v4.is_private()),
                v6.is_unspecified() || mapped.is_some_and(|v4| v4.is_unspecified()),
            )
        }
    }
}

/// Convenience filter that uses a policy with the default allowed ports and
/// the supplied private-address setting. When `allow_private` is true,
/// loopback, link-local and unspecified addresses are also allowed so that
/// the flag matches the broad meaning used in earlier code.
pub fn filter_xaddrs(xaddrs: &[String], allow_private: bool) -> Vec<String> {
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
                .validate(&Url::parse("ftp://192.0.2.1/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_disallowed_port() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://192.0.2.1:8080/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_userinfo() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://user:pass@192.0.2.1/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_empty_path_segment() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://192.0.2.1//admin").unwrap())
                .is_err()
        );
    }

    #[test]
    fn allows_root_and_trailing_slash() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://192.0.2.1/").unwrap())
                .is_ok()
        );
        assert!(
            policy
                .validate(&Url::parse("http://192.0.2.1/onvif/").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn rejects_ipv4_mapped_private() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://[::ffff:192.168.1.1]/onvif").unwrap())
                .is_err()
        );
        assert!(
            policy
                .validate(&Url::parse("http://[::ffff:169.254.169.254]/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_ipv6_unique_local() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://[fc00::1]/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn rejects_domain_names_by_default() {
        let policy = XAddrPolicy::default();
        assert!(
            policy
                .validate(&Url::parse("http://example.com/onvif").unwrap())
                .is_err()
        );
    }

    #[test]
    fn allows_domain_names_when_enabled() {
        let policy = XAddrPolicy::default().with_allow_domain_names(true);
        assert!(
            policy
                .validate(&Url::parse("http://example.com/onvif").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn filter_skips_rejected_addresses() {
        let policy = XAddrPolicy::default();
        let addrs = vec![
            "http://192.0.2.1/onvif".to_string(),
            "http://127.0.0.1/onvif".to_string(),
            "http://[::ffff:192.168.1.1]/onvif".to_string(),
            "ftp://192.0.2.1/onvif".to_string(),
        ];
        assert_eq!(
            policy.filter(&addrs),
            vec!["http://192.0.2.1/onvif".to_string()]
        );
    }

    #[test]
    fn rejects_https_to_http_redirect() {
        let policy = XAddrPolicy::default().with_allow_domain_names(true);
        let original = Url::parse("https://example.com/onvif").unwrap();
        let target = Url::parse("http://example.com/onvif").unwrap();
        assert!(policy.validate_redirect(&original, &target).is_err());
    }

    #[test]
    fn rejects_redirect_to_different_authority() {
        let policy = XAddrPolicy::default().with_allow_domain_names(true);
        let original = Url::parse("http://example.com/onvif").unwrap();
        let target = Url::parse("http://attacker.com/onvif").unwrap();
        assert!(policy.validate_redirect(&original, &target).is_err());
    }
}
