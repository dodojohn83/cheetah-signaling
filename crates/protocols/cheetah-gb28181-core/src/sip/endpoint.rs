//! Endpoint route model and NAT/`rport` send policy.
//!
//! GB28181 devices frequently sit behind NAT, so "where a peer is" cannot be
//! reduced to a single address. [`EndpointRoute`] keeps the distinct notions
//! separate and applies a deterministic policy to pick the address a server
//! should send to:
//!
//! - `observed_source`: transport source of the last accepted packet;
//! - `via_received_rport`: RFC 3581 endpoint derived from the top `Via`
//!   `received`/`rport` parameters (the NAT-mapped public address);
//! - `contact_uri`: the `Contact` URI advertised by the peer;
//! - `advertised_endpoint`: socket address parsed from `contact_uri` when its
//!   host is an IP literal (domain names cannot be resolved in this Sans-I/O
//!   core);
//! - `dialog_remote_target`: the in-dialog remote target for established
//!   dialogs.
//!
//! The module ([`crate`] consumers) stores an [`EndpointRoute`] per registered
//! device and only rewrites it from authenticated contexts; keepalive and
//! MESSAGE packets never move the send route (see [`RouteUpdateContext`] and
//! [`EndpointRoute::is_unauthenticated_drift`]).

use crate::sip::uri::SipUri;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

/// Default SIP port used when a `Contact` URI omits an explicit port.
pub const DEFAULT_SIP_PORT: u16 = 5060;

/// State of the `rport` parameter on a `Via` header (RFC 3581).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rport {
    /// No `rport` parameter is present.
    Absent,
    /// `rport` is present as a valueless flag; the client requests symmetric
    /// response routing to its observed source port.
    Requested,
    /// `rport=<port>` carries an explicit port value.
    Value(u16),
}

/// Routing-relevant parameters parsed from a top `Via` header value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViaRouteParams {
    received: Option<IpAddr>,
    rport: Rport,
}

impl ViaRouteParams {
    /// Parses the `received` and `rport` parameters from a `Via` header value.
    ///
    /// Unknown or malformed parameter values are ignored rather than causing a
    /// hard error, so a device that sends a garbled `received` still yields a
    /// usable (if degraded) result. A malformed `rport` value degrades to
    /// [`Rport::Requested`] because the presence of the token still signals
    /// that the peer wants symmetric routing.
    pub fn parse(via: &str) -> Self {
        let mut received = None;
        let mut rport = Rport::Absent;
        for token in via.split(';').skip(1) {
            let token = token.trim();
            if let Some(value) = token.strip_prefix("received=") {
                received = value.trim().parse::<IpAddr>().ok();
            } else if token.eq_ignore_ascii_case("rport") {
                rport = Rport::Requested;
            } else if let Some(value) = token.strip_prefix("rport=") {
                rport = match value.trim().parse::<u16>() {
                    Ok(port) => Rport::Value(port),
                    Err(_) => Rport::Requested,
                };
            }
        }
        Self { received, rport }
    }

    /// Returns the parsed `received` host, if present.
    pub fn received(&self) -> Option<IpAddr> {
        self.received
    }

    /// Returns the `rport` parameter state.
    pub fn rport(&self) -> Rport {
        self.rport
    }

    /// Resolves the RFC 3581 response endpoint for a request observed from
    /// `observed`.
    ///
    /// Returns `Some` only when the peer requested `rport` handling: the host
    /// is the explicit `received` value if present, otherwise the observed
    /// source IP; the port is the explicit `rport=<port>` value if present,
    /// otherwise the observed source port. When `rport` is absent the peer did
    /// not opt into symmetric routing, so `None` is returned and callers fall
    /// back to the `Contact` or observed source.
    pub fn resolved_endpoint(&self, observed: SocketAddr) -> Option<SocketAddr> {
        match self.rport {
            Rport::Absent => None,
            Rport::Requested => Some(SocketAddr::new(
                self.received.unwrap_or_else(|| observed.ip()),
                observed.port(),
            )),
            Rport::Value(port) => Some(SocketAddr::new(
                self.received.unwrap_or_else(|| observed.ip()),
                port,
            )),
        }
    }
}

/// A typed endpoint route for a GB28181 peer.
///
/// See the [module documentation](self) for the meaning of each component and
/// the send-target policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EndpointRoute {
    observed_source: SocketAddr,
    via_received_rport: Option<SocketAddr>,
    contact_uri: Option<SipUri>,
    advertised_endpoint: Option<SocketAddr>,
    dialog_remote_target: Option<SipUri>,
}

impl EndpointRoute {
    /// Builds a route from only an observed transport source, with no `Via` or
    /// `Contact` information (for example a bare in-transport request).
    pub fn from_observed(observed_source: SocketAddr) -> Self {
        Self {
            observed_source,
            via_received_rport: None,
            contact_uri: None,
            advertised_endpoint: None,
            dialog_remote_target: None,
        }
    }

    /// Builds a route from a REGISTER: the observed source, the top `Via`
    /// header value (for `received`/`rport`) and the `Contact` URI.
    pub fn from_registration(
        observed_source: SocketAddr,
        top_via: Option<&str>,
        contact_uri: Option<SipUri>,
    ) -> Self {
        let via_received_rport =
            top_via.and_then(|via| ViaRouteParams::parse(via).resolved_endpoint(observed_source));
        let advertised_endpoint = contact_uri.as_ref().and_then(socket_addr_from_uri);
        Self {
            observed_source,
            via_received_rport,
            contact_uri,
            advertised_endpoint,
            dialog_remote_target: None,
        }
    }

    /// Sets the in-dialog remote target and returns the updated route.
    #[must_use]
    pub fn with_dialog_remote_target(mut self, target: SipUri) -> Self {
        self.dialog_remote_target = Some(target);
        self
    }

    /// Replaces the in-dialog remote target.
    pub fn set_dialog_remote_target(&mut self, target: Option<SipUri>) {
        self.dialog_remote_target = target;
    }

    /// The transport source of the last accepted packet for this peer.
    pub fn observed_source(&self) -> SocketAddr {
        self.observed_source
    }

    /// The RFC 3581 `received`/`rport` endpoint, if the peer requested it.
    pub fn via_received_rport(&self) -> Option<SocketAddr> {
        self.via_received_rport
    }

    /// The advertised `Contact` URI, if any.
    pub fn contact_uri(&self) -> Option<&SipUri> {
        self.contact_uri.as_ref()
    }

    /// The socket address parsed from the `Contact` URI, if its host is an IP
    /// literal.
    pub fn advertised_endpoint(&self) -> Option<SocketAddr> {
        self.advertised_endpoint
    }

    /// The in-dialog remote target, if this route is part of a dialog.
    pub fn dialog_remote_target(&self) -> Option<&SipUri> {
        self.dialog_remote_target.as_ref()
    }

    /// Resolves the address a server should send to for out-of-dialog requests
    /// and responses.
    ///
    /// Policy: prefer the `received:rport` endpoint (NAT-mapped public address)
    /// when `rport` was requested, otherwise the `Contact` host:port when it is
    /// an IP literal, otherwise the observed source.
    pub fn send_target(&self) -> SocketAddr {
        self.via_received_rport
            .or(self.advertised_endpoint)
            .unwrap_or(self.observed_source)
    }

    /// Resolves the address to send an in-dialog request to.
    ///
    /// Uses the dialog remote target when it is an IP literal, otherwise falls
    /// back to [`Self::send_target`]. In-dialog requests must use the dialog
    /// route set/remote target rather than the REGISTER-derived endpoint.
    pub fn dialog_send_target(&self) -> SocketAddr {
        self.dialog_remote_target
            .as_ref()
            .and_then(socket_addr_from_uri)
            .unwrap_or_else(|| self.send_target())
    }

    /// Returns `true` when a packet observed from `source` in an
    /// unauthenticated context would move the established send route.
    ///
    /// This is the source-hijack signal: a keepalive/MESSAGE arriving from an
    /// address that is neither the observed source nor the resolved send target
    /// must not be allowed to rewrite the stored endpoint.
    pub fn is_unauthenticated_drift(&self, source: SocketAddr) -> bool {
        source != self.observed_source && source != self.send_target()
    }
}

/// Context in which a route update is attempted.
///
/// Per the transport design (doc §8), only authenticated REGISTER, an explicit
/// dialog target refresh, or an explicit compatibility profile may change the
/// send route. Ordinary keepalive/MESSAGE packets must never rewrite it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteUpdateContext {
    /// A successfully authenticated REGISTER (or accepted registration under an
    /// explicit challenge-optional policy).
    AuthenticatedRegister,
    /// An in-dialog target refresh (re-INVITE/2xx Contact update).
    DialogTargetRefresh,
    /// An explicitly enabled per-device compatibility profile.
    CompatibilityProfile,
    /// A keepalive received outside an authenticated context.
    UnauthenticatedKeepalive,
    /// Any other request received outside an authenticated context.
    UnauthenticatedRequest,
}

impl RouteUpdateContext {
    /// Returns `true` when this context is permitted to change the send route.
    pub fn may_change_route(self) -> bool {
        matches!(
            self,
            RouteUpdateContext::AuthenticatedRegister
                | RouteUpdateContext::DialogTargetRefresh
                | RouteUpdateContext::CompatibilityProfile
        )
    }
}

/// Parses a socket address from a `Contact`/target URI whose host is an IP
/// literal. Returns `None` for domain-name hosts, which cannot be resolved in
/// this Sans-I/O core. The port defaults to [`DEFAULT_SIP_PORT`] when absent.
pub fn socket_addr_from_uri(uri: &SipUri) -> Option<SocketAddr> {
    let host = uri.host();
    let ip = parse_host_ip(host)?;
    Some(SocketAddr::new(ip, uri.port().unwrap_or(DEFAULT_SIP_PORT)))
}

fn parse_host_ip(host: &str) -> Option<IpAddr> {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        return inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6);
    }
    host.parse::<IpAddr>().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sock(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn via_params_parse_flag_and_value_and_received() {
        let flag = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK1;rport");
        assert_eq!(flag.rport(), Rport::Requested);
        assert_eq!(flag.received(), None);

        let valued = ViaRouteParams::parse(
            "SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK1;received=203.0.113.9;rport=41234",
        );
        assert_eq!(valued.rport(), Rport::Value(41234));
        assert_eq!(valued.received(), Some("203.0.113.9".parse().unwrap()));

        let none = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060;branch=z9hG4bK1");
        assert_eq!(none.rport(), Rport::Absent);
    }

    #[test]
    fn via_params_malformed_rport_value_degrades_to_requested() {
        let params = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060;rport=notaport");
        assert_eq!(params.rport(), Rport::Requested);
    }

    #[test]
    fn resolved_endpoint_prefers_received_then_observed() {
        let observed = sock("203.0.113.9:41234");
        // Flag rport: use observed ip + observed port.
        let flag = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060;rport");
        assert_eq!(flag.resolved_endpoint(observed), Some(observed));

        // received present: use received ip + observed port.
        let recv = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060;received=198.51.100.7;rport");
        assert_eq!(
            recv.resolved_endpoint(observed),
            Some(sock("198.51.100.7:41234"))
        );

        // No rport: no symmetric endpoint.
        let none = ViaRouteParams::parse("SIP/2.0/UDP 10.0.0.1:5060");
        assert_eq!(none.resolved_endpoint(observed), None);
    }

    #[test]
    fn send_target_policy_prefers_rport_then_contact_then_observed() {
        let observed = sock("203.0.113.9:41234");
        let contact = SipUri::parse("sip:34020000001320000001@192.168.1.5:5062").unwrap();

        // rport present -> received:rport wins over contact.
        let with_rport = EndpointRoute::from_registration(
            observed,
            Some("SIP/2.0/UDP 192.168.1.5:5062;rport"),
            Some(contact.clone()),
        );
        assert_eq!(with_rport.send_target(), observed);

        // no rport, contact is an IP literal -> contact endpoint wins.
        let with_contact = EndpointRoute::from_registration(
            observed,
            Some("SIP/2.0/UDP 192.168.1.5:5062"),
            Some(contact),
        );
        assert_eq!(with_contact.send_target(), sock("192.168.1.5:5062"));

        // no rport, contact host is a domain name -> observed source.
        let domain_contact = SipUri::parse("sip:dev@device.example.com").unwrap();
        let fallback = EndpointRoute::from_registration(
            observed,
            Some("SIP/2.0/UDP device.example.com:5060"),
            Some(domain_contact),
        );
        assert_eq!(fallback.send_target(), observed);
        assert_eq!(fallback.advertised_endpoint(), None);
    }

    #[test]
    fn contact_without_port_uses_default_sip_port() {
        let contact = SipUri::parse("sip:dev@192.0.2.10").unwrap();
        assert_eq!(
            socket_addr_from_uri(&contact),
            Some(sock("192.0.2.10:5060"))
        );
    }

    #[test]
    fn ipv6_contact_literal_is_parsed() {
        let contact = SipUri::parse("sip:dev@[2001:db8::1]:5070").unwrap();
        assert_eq!(
            socket_addr_from_uri(&contact),
            Some("[2001:db8::1]:5070".parse().unwrap())
        );
    }

    #[test]
    fn dialog_send_target_prefers_dialog_remote_target() {
        let observed = sock("203.0.113.9:41234");
        let contact = SipUri::parse("sip:dev@192.168.1.5:5062").unwrap();
        let route = EndpointRoute::from_registration(
            observed,
            Some("SIP/2.0/UDP 192.168.1.5:5062;rport"),
            Some(contact),
        )
        .with_dialog_remote_target(SipUri::parse("sip:dev@198.51.100.20:5080").unwrap());
        assert_eq!(route.dialog_send_target(), sock("198.51.100.20:5080"));
        // Out-of-dialog target still follows the rport policy.
        assert_eq!(route.send_target(), observed);
    }

    #[test]
    fn unauthenticated_drift_detects_new_source() {
        let observed = sock("203.0.113.9:41234");
        let route = EndpointRoute::from_registration(
            observed,
            Some("SIP/2.0/UDP 192.168.1.5:5062;rport"),
            Some(SipUri::parse("sip:dev@192.168.1.5:5062").unwrap()),
        );
        // Same source: no drift.
        assert!(!route.is_unauthenticated_drift(observed));
        // Attacker source: drift.
        assert!(route.is_unauthenticated_drift(sock("198.51.100.66:5060")));
    }

    #[test]
    fn route_update_context_permissions() {
        assert!(RouteUpdateContext::AuthenticatedRegister.may_change_route());
        assert!(RouteUpdateContext::DialogTargetRefresh.may_change_route());
        assert!(RouteUpdateContext::CompatibilityProfile.may_change_route());
        assert!(!RouteUpdateContext::UnauthenticatedKeepalive.may_change_route());
        assert!(!RouteUpdateContext::UnauthenticatedRequest.may_change_route());
    }
}
