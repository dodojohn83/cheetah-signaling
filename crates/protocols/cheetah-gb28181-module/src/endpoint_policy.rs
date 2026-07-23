//! Outbound endpoint and network-zone policy for GB28181 (`GB4-SEC-003`).
//!
//! GB28181 platforms exchange addresses in three untrusted places: cascade
//! upstream registrar URIs, device/platform `Contact` URIs, and SDP
//! connection lines. A malicious or misconfigured peer can use any of them to
//! push the signaling node into contacting an address it must not reach
//! (SSRF), to probe the internal network, or to hijack where responses are
//! sent. This module centralises the *pure* policy that decides whether an
//! address is acceptable so that every call site applies the same rules.
//!
//! The policy is Sans-I/O: it never performs DNS resolution or socket I/O. A
//! driver resolves a host name and then hands the resolved addresses back to
//! [`EndpointPolicy::verify_resolved_addresses`] for re-verification *before*
//! and *after* connecting (DNS-rebinding defence). Redirects are rejected via
//! [`EndpointPolicy::reject_redirect`], and advertised (self) addresses are
//! constrained by [`require_explicit_advertised_host`] so a public response
//! never echoes an untrusted `Host`/`Contact`.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use cheetah_gb28181_core::{Scheme, SdpConnection, SipUri};
use cheetah_signal_types::is_internal_ip;

use crate::ingress::NetworkZone;

/// Classification of an IP address relative to network-zone boundaries.
///
/// The classes are ordered from most to least constrained. [`is_internal_ip`]
/// (from `cheetah_signal_types::net`) determines the public/internal boundary;
/// this enum additionally distinguishes the sub-zones the security doc calls
/// out (loopback, link-local and private) so a policy can admit or reject each
/// one independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkZoneClass {
    /// The unspecified address (`0.0.0.0` / `::`). Never a valid endpoint.
    Unspecified,
    /// Loopback (`127.0.0.0/8`, `::1`).
    Loopback,
    /// Link-local (`169.254.0.0/16`, `fe80::/10`).
    LinkLocal,
    /// RFC 1918 / unique-local private ranges.
    Private,
    /// Any other range flagged non-public by [`is_internal_ip`] (carrier-grade
    /// NAT, documentation, benchmarking, reserved, transition prefixes, ...).
    ReservedInternal,
    /// A globally routable public address.
    Public,
}

impl NetworkZoneClass {
    /// Classifies `ip` into exactly one zone.
    pub fn classify(ip: IpAddr) -> Self {
        if ip.is_unspecified() {
            return Self::Unspecified;
        }
        if ip.is_loopback() {
            return Self::Loopback;
        }
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_link_local() {
                    return Self::LinkLocal;
                }
                if v4.is_private() {
                    return Self::Private;
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_unicast_link_local() {
                    return Self::LinkLocal;
                }
                if v6.is_unique_local() {
                    return Self::Private;
                }
                // Map IPv4-in-IPv6 to the underlying IPv4 classification so a
                // `::ffff:10.0.0.1` cannot bypass the IPv4 private check.
                if let Some(v4) = v6.to_ipv4() {
                    return Self::classify(IpAddr::V4(v4));
                }
            }
        }
        if is_internal_ip(ip) {
            Self::ReservedInternal
        } else {
            Self::Public
        }
    }

    /// Returns `true` when the class is anything other than [`Self::Public`].
    pub fn is_internal(self) -> bool {
        !matches!(self, Self::Public)
    }
}

/// Transport permitted for an outbound SIP endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointTransport {
    /// Plain UDP.
    Udp,
    /// Plain TCP.
    Tcp,
    /// TLS over TCP.
    Tls,
}

impl EndpointTransport {
    /// Parses a SIP `transport=` parameter value (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("udp") {
            Some(Self::Udp)
        } else if value.eq_ignore_ascii_case("tcp") {
            Some(Self::Tcp)
        } else if value.eq_ignore_ascii_case("tls") {
            Some(Self::Tls)
        } else {
            None
        }
    }
}

/// The kind of host carried by an endpoint after scheme/port/zone validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EndpointHost {
    /// The host was an IP literal that already passed the zone policy.
    Ip(IpAddr),
    /// The host is a domain name. It must be resolved by the driver and every
    /// resolved address re-verified with
    /// [`EndpointPolicy::verify_resolved_addresses`] before connecting.
    DomainName(String),
}

/// Reasons an endpoint or address is rejected by the policy.
///
/// Callers must branch on the variant, never on the display string.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointPolicyError {
    /// The URI scheme is not in the allow-list.
    #[error("uri scheme is not permitted")]
    SchemeNotAllowed,
    /// The requested transport is not in the allow-list.
    #[error("transport is not permitted")]
    TransportNotAllowed,
    /// The port is zero or not in the allow-list.
    #[error("port is not permitted")]
    PortNotAllowed,
    /// The address falls into a network zone the policy rejects.
    #[error("endpoint address falls in a rejected network zone")]
    ZoneRejected,
    /// A CIDR allow-list is configured and the address is outside all entries.
    #[error("endpoint address is outside the configured allow-list")]
    CidrRejected,
    /// The SDP address type (`IP4`/`IP6`) is unsupported or disagrees with the
    /// parsed address family.
    #[error("sdp address type is not supported")]
    UnsupportedAddrType,
    /// The address literal is malformed.
    #[error("endpoint address is malformed")]
    MalformedAddress,
    /// A host name resolved to no usable addresses.
    #[error("host resolved to no addresses")]
    NoResolvedAddresses,
    /// A 3xx redirect response was received and redirects are rejected.
    #[error("redirect responses are rejected")]
    RedirectRejected,
    /// An advertised (self) address was empty, a wildcard, or otherwise not an
    /// explicitly configured routable host.
    #[error("advertised address must be an explicitly configured host")]
    AdvertisedAddressNotExplicit,
}

/// Deterministic policy for validating outbound endpoints and peer-supplied
/// addresses.
///
/// Construct with [`EndpointPolicy::builder`]. Presets:
/// [`EndpointPolicy::public_sip`] (public-only, sip/sips) and
/// [`EndpointPolicy::any_zone_sip`] (any non-unspecified zone, for private /
/// 专网 deployments where devices legitimately live on RFC 1918 networks).
#[derive(Clone, Debug)]
pub struct EndpointPolicy {
    allowed_schemes: Vec<Scheme>,
    allowed_transports: Vec<EndpointTransport>,
    allowed_ports: Option<Vec<u16>>,
    allowed_zones: Vec<NetworkZoneClass>,
    allowed_cidrs: Vec<NetworkZone>,
}

impl EndpointPolicy {
    /// Starts building a policy.
    pub fn builder() -> EndpointPolicyBuilder {
        EndpointPolicyBuilder::default()
    }

    /// A policy that accepts only public `sip`/`sips` endpoints over any
    /// transport. This is the correct default for cascade upstreams that cross
    /// a public boundary.
    pub fn public_sip() -> Self {
        Self::builder()
            .allow_scheme(Scheme::Sip)
            .allow_scheme(Scheme::Sips)
            .allow_zone(NetworkZoneClass::Public)
            .build()
    }

    /// A policy that accepts any non-[`NetworkZoneClass::Unspecified`] zone
    /// over `sip`/`sips`. Suitable for private / 专网 deployments; still rejects
    /// the unspecified address, which is never a valid endpoint.
    pub fn any_zone_sip() -> Self {
        Self::builder()
            .allow_scheme(Scheme::Sip)
            .allow_scheme(Scheme::Sips)
            .allow_zone(NetworkZoneClass::Loopback)
            .allow_zone(NetworkZoneClass::LinkLocal)
            .allow_zone(NetworkZoneClass::Private)
            .allow_zone(NetworkZoneClass::ReservedInternal)
            .allow_zone(NetworkZoneClass::Public)
            .build()
    }

    /// Validates a resolved IP address against the zone and CIDR rules.
    pub fn validate_ip(&self, ip: IpAddr) -> Result<(), EndpointPolicyError> {
        let class = NetworkZoneClass::classify(ip);
        if !self.allowed_zones.contains(&class) {
            return Err(EndpointPolicyError::ZoneRejected);
        }
        if !self.allowed_cidrs.is_empty() && !self.allowed_cidrs.iter().any(|z| z.contains(ip)) {
            return Err(EndpointPolicyError::CidrRejected);
        }
        Ok(())
    }

    /// Validates a port against the allow-list. Port `0` is always rejected.
    pub fn validate_port(&self, port: u16) -> Result<(), EndpointPolicyError> {
        if port == 0 {
            return Err(EndpointPolicyError::PortNotAllowed);
        }
        match &self.allowed_ports {
            Some(ports) if !ports.contains(&port) => Err(EndpointPolicyError::PortNotAllowed),
            _ => Ok(()),
        }
    }

    /// Validates a SIP endpoint URI (scheme, `transport=` parameter, port and,
    /// when the host is an IP literal, its network zone).
    ///
    /// Returns the classified [`EndpointHost`]: an [`EndpointHost::Ip`] has
    /// already passed the zone check, while an [`EndpointHost::DomainName`]
    /// must be resolved and re-verified by the driver before connecting.
    pub fn validate_sip_endpoint(&self, uri: &SipUri) -> Result<EndpointHost, EndpointPolicyError> {
        if !self.allowed_schemes.contains(&uri.scheme()) {
            return Err(EndpointPolicyError::SchemeNotAllowed);
        }
        self.validate_transport_param(uri)?;
        // A `sips:` URI without an explicit port defaults to 5061, otherwise
        // 5060; we only enforce the allow-list when a port is present or a
        // non-empty allow-list is configured, defaulting an absent port to the
        // scheme default so the check is deterministic.
        let port = uri.port().unwrap_or(default_port(uri.scheme()));
        self.validate_port(port)?;

        match parse_host_ip(uri.host()) {
            Some(ip) => {
                self.validate_ip(ip)?;
                Ok(EndpointHost::Ip(ip))
            }
            None => {
                if uri.host().trim().is_empty() {
                    return Err(EndpointPolicyError::MalformedAddress);
                }
                Ok(EndpointHost::DomainName(uri.host().to_string()))
            }
        }
    }

    fn validate_transport_param(&self, uri: &SipUri) -> Result<(), EndpointPolicyError> {
        let Some(transport) = uri.parameters().iter().find_map(|(k, v)| {
            if k.eq_ignore_ascii_case("transport") {
                v.as_deref()
            } else {
                None
            }
        }) else {
            return Ok(());
        };
        match EndpointTransport::parse(transport) {
            Some(t) if self.allowed_transports.contains(&t) => Ok(()),
            _ => Err(EndpointPolicyError::TransportNotAllowed),
        }
    }

    /// Validates an SDP connection line and media port.
    ///
    /// The address must be a well-formed unicast IP literal (SDP `c=` for
    /// GB28181 media is always a numeric address, never a name), its family
    /// must match the declared `IP4`/`IP6` address type, and it must satisfy
    /// the zone policy. Multicast group notation and the unspecified address
    /// are rejected because they are never valid unicast media targets.
    pub fn validate_sdp_connection(
        &self,
        connection: &SdpConnection,
        port: u16,
    ) -> Result<IpAddr, EndpointPolicyError> {
        self.validate_port(port)?;
        // Strip any multicast TTL / address-count suffix (`addr/ttl[/count]`),
        // which is not a plain unicast target.
        if connection.address.contains('/') {
            return Err(EndpointPolicyError::MalformedAddress);
        }
        let ip = parse_host_ip(connection.address.trim())
            .ok_or(EndpointPolicyError::MalformedAddress)?;
        let matches_type = matches!(
            (connection.addrtype.to_ascii_uppercase().as_str(), ip),
            ("IP4", IpAddr::V4(_)) | ("IP6", IpAddr::V6(_))
        );
        if !matches_type {
            return Err(EndpointPolicyError::UnsupportedAddrType);
        }
        self.validate_ip(ip)?;
        Ok(ip)
    }

    /// Re-verifies the addresses a host name resolved to.
    ///
    /// The driver calls this after DNS resolution (and again with the connected
    /// peer address) so a name that resolved to a public address at policy
    /// time cannot later be rebound to an internal one. The set must be
    /// non-empty and *every* address must satisfy the policy: a name that
    /// resolves to a mix of public and internal addresses is rejected outright.
    pub fn verify_resolved_addresses(
        &self,
        addresses: &[IpAddr],
    ) -> Result<(), EndpointPolicyError> {
        if addresses.is_empty() {
            return Err(EndpointPolicyError::NoResolvedAddresses);
        }
        for ip in addresses {
            self.validate_ip(*ip)?;
        }
        Ok(())
    }

    /// Rejects redirect (3xx) status codes.
    ///
    /// Following a redirect would let an upstream point the cascade at an
    /// arbitrary endpoint, so redirects are never followed. Non-3xx codes pass
    /// through unchanged.
    pub fn reject_redirect(status_code: u16) -> Result<(), EndpointPolicyError> {
        if (300..400).contains(&status_code) {
            Err(EndpointPolicyError::RedirectRejected)
        } else {
            Ok(())
        }
    }
}

/// Requires that an advertised (self) address is an explicitly configured,
/// routable host rather than a value copied from an untrusted `Host`/`Contact`
/// header.
///
/// An empty host, or an IP-literal host that is unspecified (`0.0.0.0` / `::`),
/// is rejected. Domain names and concrete IP literals are accepted; the point
/// is that the operator configured *something explicit* rather than the node
/// echoing a wildcard bind or an attacker-chosen header.
pub fn require_explicit_advertised_host(host: &str) -> Result<(), EndpointPolicyError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(EndpointPolicyError::AdvertisedAddressNotExplicit);
    }
    if let Some(ip) = parse_host_ip(host)
        && ip.is_unspecified()
    {
        return Err(EndpointPolicyError::AdvertisedAddressNotExplicit);
    }
    Ok(())
}

/// Parses a bare host into an [`IpAddr`], accepting bracketed IPv6 literals
/// (`[2001:db8::1]`). Returns `None` for domain names.
pub fn parse_host_ip(host: &str) -> Option<IpAddr> {
    let host = host.trim();
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    if let Ok(v4) = Ipv4Addr::from_str(host) {
        return Some(IpAddr::V4(v4));
    }
    Ipv6Addr::from_str(host).ok().map(IpAddr::V6)
}

fn default_port(scheme: Scheme) -> u16 {
    match scheme {
        Scheme::Sip => 5060,
        Scheme::Sips => 5061,
    }
}

/// Builder for [`EndpointPolicy`].
#[derive(Clone, Debug, Default)]
pub struct EndpointPolicyBuilder {
    allowed_schemes: Vec<Scheme>,
    allowed_transports: Vec<EndpointTransport>,
    allowed_ports: Option<Vec<u16>>,
    allowed_zones: Vec<NetworkZoneClass>,
    allowed_cidrs: Vec<NetworkZone>,
}

impl EndpointPolicyBuilder {
    /// Adds an allowed URI scheme.
    #[must_use]
    pub fn allow_scheme(mut self, scheme: Scheme) -> Self {
        if !self.allowed_schemes.contains(&scheme) {
            self.allowed_schemes.push(scheme);
        }
        self
    }

    /// Adds an allowed transport.
    #[must_use]
    pub fn allow_transport(mut self, transport: EndpointTransport) -> Self {
        if !self.allowed_transports.contains(&transport) {
            self.allowed_transports.push(transport);
        }
        self
    }

    /// Adds an allowed network-zone class.
    #[must_use]
    pub fn allow_zone(mut self, zone: NetworkZoneClass) -> Self {
        if !self.allowed_zones.contains(&zone) {
            self.allowed_zones.push(zone);
        }
        self
    }

    /// Restricts the endpoint to a set of ports. Without this, any non-zero
    /// port is accepted.
    #[must_use]
    pub fn allow_ports(mut self, ports: Vec<u16>) -> Self {
        self.allowed_ports = Some(ports);
        self
    }

    /// Adds a CIDR block to the allow-list. When any CIDR is configured, an
    /// address must fall inside one of them in addition to passing the zone
    /// check.
    #[must_use]
    pub fn allow_cidr(mut self, zone: NetworkZone) -> Self {
        self.allowed_cidrs.push(zone);
        self
    }

    /// Builds the policy. Transports default to UDP/TCP/TLS when none were
    /// specified so a caller that only cares about schemes/zones is not forced
    /// to enumerate them.
    pub fn build(mut self) -> EndpointPolicy {
        if self.allowed_transports.is_empty() {
            self.allowed_transports = vec![
                EndpointTransport::Udp,
                EndpointTransport::Tcp,
                EndpointTransport::Tls,
            ];
        }
        EndpointPolicy {
            allowed_schemes: self.allowed_schemes,
            allowed_transports: self.allowed_transports,
            allowed_ports: self.allowed_ports,
            allowed_zones: self.allowed_zones,
            allowed_cidrs: self.allowed_cidrs,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn public_v4() -> IpAddr {
        "8.8.8.8".parse().unwrap()
    }

    #[test]
    fn classify_covers_zone_boundaries() {
        assert_eq!(
            NetworkZoneClass::classify("0.0.0.0".parse().unwrap()),
            NetworkZoneClass::Unspecified
        );
        assert_eq!(
            NetworkZoneClass::classify("127.0.0.1".parse().unwrap()),
            NetworkZoneClass::Loopback
        );
        assert_eq!(
            NetworkZoneClass::classify("169.254.1.1".parse().unwrap()),
            NetworkZoneClass::LinkLocal
        );
        assert_eq!(
            NetworkZoneClass::classify("10.1.2.3".parse().unwrap()),
            NetworkZoneClass::Private
        );
        assert_eq!(
            NetworkZoneClass::classify("100.64.0.1".parse().unwrap()),
            NetworkZoneClass::ReservedInternal
        );
        assert_eq!(
            NetworkZoneClass::classify(public_v4()),
            NetworkZoneClass::Public
        );
        assert_eq!(
            NetworkZoneClass::classify("::1".parse().unwrap()),
            NetworkZoneClass::Loopback
        );
        assert_eq!(
            NetworkZoneClass::classify("fc00::1".parse().unwrap()),
            NetworkZoneClass::Private
        );
    }

    #[test]
    fn ipv4_mapped_ipv6_is_classified_by_inner_v4() {
        assert_eq!(
            NetworkZoneClass::classify("::ffff:10.0.0.1".parse().unwrap()),
            NetworkZoneClass::Private
        );
    }

    #[test]
    fn public_sip_policy_accepts_public_ip_literal_endpoint() {
        let policy = EndpointPolicy::public_sip();
        let uri = SipUri::parse("sip:platform@8.8.8.8:5060").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&uri).unwrap(),
            EndpointHost::Ip(public_v4())
        );
    }

    #[test]
    fn public_sip_policy_rejects_internal_ip_literal_endpoint() {
        let policy = EndpointPolicy::public_sip();
        let uri = SipUri::parse("sip:platform@192.168.1.10:5060").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&uri),
            Err(EndpointPolicyError::ZoneRejected)
        );
    }

    #[test]
    fn domain_name_endpoint_defers_to_dns_reverification() {
        let policy = EndpointPolicy::public_sip();
        let uri = SipUri::parse("sip:platform@registrar.example.com:5060").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&uri).unwrap(),
            EndpointHost::DomainName("registrar.example.com".to_string())
        );
    }

    #[test]
    fn scheme_and_transport_are_enforced() {
        let policy = EndpointPolicy::builder()
            .allow_scheme(Scheme::Sips)
            .allow_transport(EndpointTransport::Tls)
            .allow_zone(NetworkZoneClass::Public)
            .build();
        // `sip:` scheme rejected.
        let sip = SipUri::parse("sip:p@8.8.8.8:5061").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&sip),
            Err(EndpointPolicyError::SchemeNotAllowed)
        );
        // `sips:` with disallowed transport rejected.
        let udp = SipUri::parse("sips:p@8.8.8.8:5061;transport=udp").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&udp),
            Err(EndpointPolicyError::TransportNotAllowed)
        );
        // `sips:` with allowed transport accepted.
        let tls = SipUri::parse("sips:p@8.8.8.8:5061;transport=tls").unwrap();
        assert!(policy.validate_sip_endpoint(&tls).is_ok());
    }

    #[test]
    fn port_allow_list_is_enforced() {
        let policy = EndpointPolicy::builder()
            .allow_scheme(Scheme::Sip)
            .allow_zone(NetworkZoneClass::Public)
            .allow_ports(vec![5060])
            .build();
        let ok = SipUri::parse("sip:p@8.8.8.8:5060").unwrap();
        assert!(policy.validate_sip_endpoint(&ok).is_ok());
        let bad = SipUri::parse("sip:p@8.8.8.8:5080").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&bad),
            Err(EndpointPolicyError::PortNotAllowed)
        );
    }

    #[test]
    fn cidr_allow_list_further_restricts_public_addresses() {
        let policy = EndpointPolicy::builder()
            .allow_scheme(Scheme::Sip)
            .allow_zone(NetworkZoneClass::Public)
            .allow_cidr(NetworkZone::parse("8.8.8.0/24").unwrap())
            .build();
        let inside = SipUri::parse("sip:p@8.8.8.8:5060").unwrap();
        assert!(policy.validate_sip_endpoint(&inside).is_ok());
        let outside = SipUri::parse("sip:p@1.1.1.1:5060").unwrap();
        assert_eq!(
            policy.validate_sip_endpoint(&outside),
            Err(EndpointPolicyError::CidrRejected)
        );
    }

    #[test]
    fn sdp_connection_validation_matches_family_and_zone() {
        let policy = EndpointPolicy::any_zone_sip();
        let conn = SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: "192.168.1.50".to_string(),
        };
        assert_eq!(
            policy.validate_sdp_connection(&conn, 10000).unwrap(),
            "192.168.1.50".parse::<IpAddr>().unwrap()
        );
        // Family mismatch.
        let mismatch = SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP6".to_string(),
            address: "192.168.1.50".to_string(),
        };
        assert_eq!(
            policy.validate_sdp_connection(&mismatch, 10000),
            Err(EndpointPolicyError::UnsupportedAddrType)
        );
        // Multicast/suffix notation rejected.
        let multicast = SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: "239.0.0.1/16".to_string(),
        };
        assert_eq!(
            policy.validate_sdp_connection(&multicast, 10000),
            Err(EndpointPolicyError::MalformedAddress)
        );
    }

    #[test]
    fn sdp_public_policy_rejects_internal_media_address() {
        let policy = EndpointPolicy::public_sip();
        let conn = SdpConnection {
            nettype: "IN".to_string(),
            addrtype: "IP4".to_string(),
            address: "127.0.0.1".to_string(),
        };
        assert_eq!(
            policy.validate_sdp_connection(&conn, 10000),
            Err(EndpointPolicyError::ZoneRejected)
        );
    }

    #[test]
    fn dns_reverification_rejects_any_internal_address() {
        let policy = EndpointPolicy::public_sip();
        assert!(policy.verify_resolved_addresses(&[public_v4()]).is_ok());
        // Mixed set with an internal address is rejected outright.
        assert_eq!(
            policy.verify_resolved_addresses(&[public_v4(), "10.0.0.1".parse().unwrap()]),
            Err(EndpointPolicyError::ZoneRejected)
        );
        assert_eq!(
            policy.verify_resolved_addresses(&[]),
            Err(EndpointPolicyError::NoResolvedAddresses)
        );
    }

    #[test]
    fn redirects_are_rejected() {
        assert_eq!(
            EndpointPolicy::reject_redirect(302),
            Err(EndpointPolicyError::RedirectRejected)
        );
        assert!(EndpointPolicy::reject_redirect(200).is_ok());
        assert!(EndpointPolicy::reject_redirect(401).is_ok());
    }

    #[test]
    fn advertised_host_must_be_explicit() {
        assert!(require_explicit_advertised_host("sip.example.com").is_ok());
        assert!(require_explicit_advertised_host("8.8.8.8").is_ok());
        assert_eq!(
            require_explicit_advertised_host(""),
            Err(EndpointPolicyError::AdvertisedAddressNotExplicit)
        );
        assert_eq!(
            require_explicit_advertised_host("0.0.0.0"),
            Err(EndpointPolicyError::AdvertisedAddressNotExplicit)
        );
        assert_eq!(
            require_explicit_advertised_host("::"),
            Err(EndpointPolicyError::AdvertisedAddressNotExplicit)
        );
    }
}
