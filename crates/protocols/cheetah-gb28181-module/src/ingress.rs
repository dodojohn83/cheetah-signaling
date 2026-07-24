//! Listener-tenant routing, body-identity and endpoint-security validation for
//! GB28181 access (`GB4-ACC-003`).
//!
//! [`AccessIngress`] sits in front of the persistent [`ProtocolSessionLink`]
//! ([`crate::session`]) and turns *untrusted* wire facts into a *trusted*
//! [`SessionContext`] before any domain side effect is applied:
//!
//! 1. **Listener tenant resolution.** The Request-URI / To domain is matched
//!    against the configured [`ListenerBinding`]s. An unconfigured domain is
//!    rejected as `404`; a domain that resolves to more than one listener (or a
//!    Request-URI / To pair that disagrees) is rejected as `403`. The tenant
//!    and local identity are taken from the resolved listener, never trusted
//!    from the caller.
//! 2. **Body identity.** For REGISTER / Keepalive / MESSAGE the MANSCDP body
//!    `DeviceID` must match the From (or, when From is absent, the To) URI user
//!    part, so a device cannot report state under another device's identity.
//! 3. **Endpoint security.** Only an *authenticated* REGISTER (or an in-dialog
//!    target refresh) may rewrite the stored endpoint. Keepalive and MESSAGE
//!    never rewrite the route, which prevents an off-path source from hijacking
//!    a device's binding.
//! 4. **Network zones.** When a listener declares allowed zones, the observed
//!    source address must fall inside one of them or the request is rejected as
//!    `403`.
//!
//! # Layering
//!
//! The ingress is Sans-I/O: it performs no socket, database, NATS or media I/O
//! itself. Persistence goes through the injected
//! [`ProtocolSessionRepository`](cheetah_domain::ProtocolSessionRepository)
//! port via [`ProtocolSessionLink`], so the module stays within layer four.

use std::net::IpAddr;

use cheetah_domain::{
    CompatibilityProfile, LocalIdentity, ProtocolSessionRepository, SipTransport,
};
use cheetah_signal_types::{DeviceId, NodeId, OwnerEpoch, ProtocolIdentity, TenantId, clamp_str};

use crate::session::{
    ProtocolSessionLink, RegisterOutcome, RegisterParams, SessionContext, SessionLinkError,
};

/// Maximum accepted byte length of a domain or user-part string.
const MAX_IDENTITY_BYTES: usize = 253;
/// Maximum byte length of a CIDR string passed to [`NetworkZone::parse`].
/// The longest valid textual CIDR (`IPv6` with scope id) is well under this.
const MAX_CIDR_BYTES: usize = 128;
/// Maximum byte length of an `IngressConfigError` diagnostic string.
const MAX_INGRESS_CONFIG_ERROR_BYTES: usize = 512;

/// SIP method whose access is being validated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngressMethod {
    /// A REGISTER (create / refresh / unregister) request.
    Register,
    /// A Keepalive MESSAGE.
    Keepalive,
    /// Any other MANSCDP MESSAGE (catalog, notify, control response, ...).
    Message,
}

/// Build-time configuration error for the ingress routing table.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngressConfigError {
    /// A listener field was empty.
    #[error("listener field must not be empty: {0}")]
    EmptyField(&'static str),
    /// A listener field exceeded the maximum length.
    #[error("listener field too long: {0}")]
    FieldTooLong(&'static str),
    /// A network zone could not be parsed as a CIDR block.
    #[error("invalid network zone: {0}")]
    InvalidZone(String),
}

impl IngressConfigError {
    /// Creates an `InvalidZone` error with a bounded copy of `cidr` so the
    /// diagnostic cannot carry an arbitrary-sized input.
    pub fn invalid_zone(cidr: &str) -> Self {
        Self::InvalidZone(clamp_str(cidr, MAX_INGRESS_CONFIG_ERROR_BYTES))
    }
}

/// Validation / routing error produced while admitting a request.
///
/// Each variant maps to a stable SIP status via [`IngressError::sip_status`];
/// callers must not branch on the display string.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngressError {
    /// No listener is configured for the request domain (`404`).
    #[error("no listener is configured for the request domain")]
    UnconfiguredDomain,
    /// The request domain resolves to more than one listener, or the
    /// Request-URI and To domains disagree (`403`).
    #[error("request domain is ambiguous")]
    AmbiguousDomain,
    /// The MANSCDP body `DeviceID` does not match the From/To URI user (`403`).
    #[error("body device identity does not match the request URI")]
    BodyIdentityMismatch,
    /// The observed source address is outside the listener's allowed zones
    /// (`403`).
    #[error("source address is outside the allowed network zones")]
    SourceZoneRejected,
    /// An endpoint-updating request arrived without authentication (`401`).
    #[error("endpoint update requires an authenticated REGISTER")]
    AuthenticationRequired,
    /// A request tried to rewrite the endpoint through a path that is not
    /// allowed to (keepalive / MESSAGE) (`403`).
    #[error("this request may not rewrite the stored endpoint")]
    EndpointUpdateForbidden,
    /// A persistence / fencing error from the session link.
    #[error(transparent)]
    Session(#[from] SessionLinkError),
}

impl IngressError {
    /// Stable SIP status code for the rejection.
    pub fn sip_status(&self) -> u16 {
        match self {
            Self::UnconfiguredDomain => 404,
            Self::AmbiguousDomain
            | Self::BodyIdentityMismatch
            | Self::SourceZoneRejected
            | Self::EndpointUpdateForbidden => 403,
            Self::AuthenticationRequired => 401,
            Self::Session(SessionLinkError::NotRegistered | SessionLinkError::Expired) => 403,
            Self::Session(SessionLinkError::StaleOwner { .. }) => 403,
            Self::Session(SessionLinkError::Repository(_)) => 500,
        }
    }
}

/// A CIDR network zone used to constrain the observed source address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkZone {
    base: IpAddr,
    prefix_len: u8,
}

impl NetworkZone {
    /// Parses a CIDR block such as `203.0.113.0/24` or `2001:db8::/32`.
    ///
    /// The prefix length must be within the address family's range (`0..=32`
    /// for IPv4, `0..=128` for IPv6). Inputs longer than [`MAX_CIDR_BYTES`]
    /// are rejected early to avoid allocating a huge diagnostic on failure.
    pub fn parse(cidr: &str) -> Result<Self, IngressConfigError> {
        if cidr.len() > MAX_CIDR_BYTES {
            return Err(IngressConfigError::invalid_zone(cidr));
        }
        let (addr, prefix) = cidr
            .split_once('/')
            .ok_or_else(|| IngressConfigError::invalid_zone(cidr))?;
        let base: IpAddr = addr
            .parse()
            .map_err(|_| IngressConfigError::invalid_zone(cidr))?;
        let prefix_len: u8 = prefix
            .parse()
            .map_err(|_| IngressConfigError::invalid_zone(cidr))?;
        let max = match base {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix_len > max {
            return Err(IngressConfigError::invalid_zone(cidr));
        }
        Ok(Self { base, prefix_len })
    }

    /// Returns `true` when `ip` falls inside the zone.
    ///
    /// Addresses of a different family than the zone base never match.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.base, ip) {
            (IpAddr::V4(base), IpAddr::V4(ip)) => {
                prefix_matches(&base.octets(), &ip.octets(), self.prefix_len)
            }
            (IpAddr::V6(base), IpAddr::V6(ip)) => {
                prefix_matches(&base.octets(), &ip.octets(), self.prefix_len)
            }
            _ => false,
        }
    }
}

/// Compares the first `prefix_len` bits of two equal-length byte arrays.
fn prefix_matches(base: &[u8], candidate: &[u8], prefix_len: u8) -> bool {
    let mut remaining = usize::from(prefix_len);
    for (b, c) in base.iter().zip(candidate.iter()) {
        if remaining == 0 {
            break;
        }
        if remaining >= 8 {
            if b != c {
                return false;
            }
            remaining -= 8;
        } else {
            let mask = 0xffu8 << (8 - remaining);
            return (b & mask) == (c & mask);
        }
    }
    true
}

/// One configured listener: a SIP domain mapped to a tenant and local identity.
///
/// Fields are private so that a binding cannot be constructed with an empty
/// domain, tenant or identity; use [`ListenerBinding::new`].
#[derive(Clone, Debug)]
pub struct ListenerBinding {
    domain: String,
    tenant_id: TenantId,
    local_identity: LocalIdentity,
    allowed_zones: Vec<NetworkZone>,
}

impl ListenerBinding {
    /// Creates a listener binding for `domain` mapped to `tenant_id`.
    ///
    /// The `domain` must be non-empty and the `local_identity` must carry a
    /// matching, non-empty domain. Returns [`IngressConfigError`] otherwise.
    pub fn new(
        domain: impl Into<String>,
        tenant_id: TenantId,
        local_identity: LocalIdentity,
    ) -> Result<Self, IngressConfigError> {
        let domain = domain.into();
        check_identity("domain", &domain)?;
        check_identity("listener_id", &local_identity.listener_id)?;
        check_identity("local_device_id", &local_identity.local_device_id)?;
        check_identity("realm", &local_identity.realm)?;
        Ok(Self {
            domain,
            tenant_id,
            local_identity,
            allowed_zones: Vec::new(),
        })
    }

    /// Returns a copy of the binding constrained to `zones`.
    ///
    /// An empty zone list (the default) admits any source address.
    #[must_use]
    pub fn with_allowed_zones(mut self, zones: Vec<NetworkZone>) -> Self {
        self.allowed_zones = zones;
        self
    }

    /// SIP domain matched against the Request-URI / To host.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Tenant the listener admits devices into.
    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    /// Local listener identity recorded on sessions created here.
    pub fn local_identity(&self) -> &LocalIdentity {
        &self.local_identity
    }

    /// Rejects `source` when zones are configured and none contain it.
    fn admit_source(&self, source: IpAddr) -> Result<(), IngressError> {
        if self.allowed_zones.is_empty()
            || self.allowed_zones.iter().any(|zone| zone.contains(source))
        {
            Ok(())
        } else {
            Err(IngressError::SourceZoneRejected)
        }
    }
}

fn check_identity(name: &'static str, value: &str) -> Result<(), IngressConfigError> {
    if value.is_empty() {
        return Err(IngressConfigError::EmptyField(name));
    }
    if value.len() > MAX_IDENTITY_BYTES {
        return Err(IngressConfigError::FieldTooLong(name));
    }
    Ok(())
}

/// Untrusted per-request wire facts used for validation.
///
/// The user parts are the SIP URI user components (the GB device id), not the
/// full addresses; the caller extracts them from the From / To headers.
#[derive(Clone, Debug)]
pub struct RequestIdentity {
    /// Host of the Request-URI.
    pub request_uri_host: String,
    /// Host of the To URI.
    pub to_host: String,
    /// User part of the From URI (the reporting device).
    pub from_user: String,
    /// User part of the To URI.
    pub to_user: String,
    /// `DeviceID` carried in the MANSCDP body, if any.
    pub body_device_id: Option<String>,
    /// Source address observed on the received packet.
    pub observed_source: IpAddr,
}

/// Trusted device / ownership facts resolved by the application shard.
///
/// The tenant and local identity are intentionally *not* part of this struct:
/// they are supplied by the resolved [`ListenerBinding`], so a caller cannot
/// smuggle a device into a tenant it did not authenticate against.
#[derive(Clone, Debug)]
pub struct DeviceBinding {
    /// Internal device identifier.
    pub device_id: DeviceId,
    /// External GB28181 device identity.
    pub protocol_identity: ProtocolIdentity,
    /// SIP transport carrying the request.
    pub transport: SipTransport,
    /// Node currently owning the device's session.
    pub owner_node_id: NodeId,
    /// Owner epoch fencing the session.
    pub owner_epoch: OwnerEpoch,
    /// Compatibility profile applied to the device.
    pub compatibility: CompatibilityProfile,
}

/// Front door that resolves tenant, validates identity/endpoint security and
/// then drives the persistent [`ProtocolSessionLink`].
#[derive(Clone, Debug)]
pub struct AccessIngress {
    listeners: Vec<ListenerBinding>,
}

impl AccessIngress {
    /// Creates an ingress over the configured listeners.
    ///
    /// Duplicate domains are permitted so that a mis-configuration is surfaced
    /// as a deterministic [`IngressError::AmbiguousDomain`] at request time
    /// rather than silently routing to an arbitrary tenant.
    pub fn new(listeners: Vec<ListenerBinding>) -> Self {
        Self { listeners }
    }

    /// Resolves the listener for a request, enforcing that the Request-URI and
    /// To domains agree and map to exactly one listener.
    pub fn resolve_listener(
        &self,
        ident: &RequestIdentity,
    ) -> Result<&ListenerBinding, IngressError> {
        let by_request = self.match_domain(&ident.request_uri_host)?;
        // The To domain, when present, must resolve to the same listener.
        if !ident.to_host.is_empty() && !ident.to_host.eq_ignore_ascii_case(&ident.request_uri_host)
        {
            let by_to = self.match_domain(&ident.to_host)?;
            if !std::ptr::eq(by_request, by_to) {
                return Err(IngressError::AmbiguousDomain);
            }
        }
        Ok(by_request)
    }

    /// Matches a single host against the listener table.
    fn match_domain(&self, host: &str) -> Result<&ListenerBinding, IngressError> {
        if host.is_empty() {
            return Err(IngressError::UnconfiguredDomain);
        }
        let mut matches = self
            .listeners
            .iter()
            .filter(|listener| listener.domain.eq_ignore_ascii_case(host));
        let Some(first) = matches.next() else {
            return Err(IngressError::UnconfiguredDomain);
        };
        if matches.next().is_some() {
            return Err(IngressError::AmbiguousDomain);
        }
        Ok(first)
    }

    /// Validates tenant routing, body identity and source zone for a request,
    /// returning the resolved listener.
    ///
    /// This is the read-only admission check shared by every method; it applies
    /// no side effects and is suitable for MESSAGE handling that persists
    /// nothing itself.
    pub fn admit(
        &self,
        ident: &RequestIdentity,
        method: IngressMethod,
    ) -> Result<&ListenerBinding, IngressError> {
        let listener = self.resolve_listener(ident)?;
        listener.admit_source(ident.observed_source)?;
        validate_body_identity(method, ident)?;
        Ok(listener)
    }

    /// Admits and persists an authenticated REGISTER.
    ///
    /// `authenticated` must be `true`: only an authenticated REGISTER may
    /// create a session or rewrite its endpoint. The tenant and local identity
    /// are taken from the resolved listener, so the endpoint is only ever
    /// written on this path.
    pub async fn register(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        link: &ProtocolSessionLink,
        ident: &RequestIdentity,
        binding: &DeviceBinding,
        params: RegisterParams,
        authenticated: bool,
    ) -> Result<RegisterOutcome, IngressError> {
        if !authenticated {
            return Err(IngressError::AuthenticationRequired);
        }
        let ctx = {
            let listener = self.admit(ident, IngressMethod::Register)?;
            self.context(listener, binding)
        };
        Ok(link.register(repo, &ctx, params).await?)
    }

    /// Admits and applies a Keepalive.
    ///
    /// A keepalive never rewrites the stored endpoint, so a keepalive observed
    /// from a different source cannot hijack the device's route. Cross-tenant
    /// keepalives resolve to a tenant with no session for the device and are
    /// rejected as [`SessionLinkError::NotRegistered`].
    pub async fn keepalive(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        link: &ProtocolSessionLink,
        ident: &RequestIdentity,
        binding: &DeviceBinding,
    ) -> Result<(), IngressError> {
        let ctx = {
            let listener = self.admit(ident, IngressMethod::Keepalive)?;
            self.context(listener, binding)
        };
        link.keepalive(repo, &ctx).await?;
        Ok(())
    }

    /// Admits and applies an explicit unregister (`Expires=0`).
    pub async fn unregister(
        &self,
        repo: &mut dyn ProtocolSessionRepository,
        link: &ProtocolSessionLink,
        ident: &RequestIdentity,
        binding: &DeviceBinding,
        authenticated: bool,
    ) -> Result<Option<cheetah_signal_types::ProtocolSessionId>, IngressError> {
        if !authenticated {
            return Err(IngressError::AuthenticationRequired);
        }
        let ctx = {
            let listener = self.admit(ident, IngressMethod::Register)?;
            self.context(listener, binding)
        };
        Ok(link.unregister(repo, &ctx).await?)
    }

    /// Rejects any attempt to rewrite the endpoint from a non-REGISTER path.
    ///
    /// Keepalive and MESSAGE requests must not carry an endpoint update; the
    /// only paths permitted to write the route are an authenticated REGISTER
    /// and an in-dialog target refresh, both of which are represented by
    /// [`IngressMethod::Register`].
    pub fn authorize_endpoint_update(
        method: IngressMethod,
        authenticated: bool,
        in_dialog_refresh: bool,
    ) -> Result<(), IngressError> {
        match method {
            IngressMethod::Register if authenticated || in_dialog_refresh => Ok(()),
            IngressMethod::Register => Err(IngressError::AuthenticationRequired),
            IngressMethod::Keepalive | IngressMethod::Message => {
                Err(IngressError::EndpointUpdateForbidden)
            }
        }
    }

    /// Builds a trusted [`SessionContext`] from the resolved listener and the
    /// application-supplied device / ownership facts.
    fn context(&self, listener: &ListenerBinding, binding: &DeviceBinding) -> SessionContext {
        SessionContext {
            tenant_id: listener.tenant_id,
            device_id: binding.device_id,
            protocol_identity: binding.protocol_identity.clone(),
            local_identity: listener.local_identity.clone(),
            transport: binding.transport,
            owner_node_id: binding.owner_node_id,
            owner_epoch: binding.owner_epoch,
            compatibility: binding.compatibility.clone(),
        }
    }
}

/// Validates that the body `DeviceID` matches the request identity.
///
/// The expected identity is the From user, falling back to the To user when
/// From is absent. REGISTER may omit the body (no MANSCDP payload); Keepalive
/// and MESSAGE must carry a matching `DeviceID`.
fn validate_body_identity(
    method: IngressMethod,
    ident: &RequestIdentity,
) -> Result<(), IngressError> {
    let expected = if !ident.from_user.is_empty() {
        ident.from_user.as_str()
    } else if !ident.to_user.is_empty() {
        ident.to_user.as_str()
    } else {
        return Err(IngressError::BodyIdentityMismatch);
    };

    match (&ident.body_device_id, method) {
        (Some(body), _) => {
            if body == expected {
                Ok(())
            } else {
                Err(IngressError::BodyIdentityMismatch)
            }
        }
        (None, IngressMethod::Register) => Ok(()),
        (None, IngressMethod::Keepalive | IngressMethod::Message) => {
            Err(IngressError::BodyIdentityMismatch)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn network_zone_matches_ipv4_prefix() {
        let zone = NetworkZone::parse("203.0.113.0/24").unwrap();
        assert!(zone.contains("203.0.113.200".parse().unwrap()));
        assert!(!zone.contains("203.0.114.1".parse().unwrap()));
    }

    #[test]
    fn network_zone_matches_ipv6_prefix() {
        let zone = NetworkZone::parse("2001:db8::/32").unwrap();
        assert!(zone.contains("2001:db8:1234::1".parse().unwrap()));
        assert!(!zone.contains("2001:db9::1".parse().unwrap()));
    }

    #[test]
    fn network_zone_rejects_cross_family() {
        let zone = NetworkZone::parse("203.0.113.0/24").unwrap();
        assert!(!zone.contains("::1".parse().unwrap()));
    }

    #[test]
    fn network_zone_rejects_oversized_prefix() {
        assert!(NetworkZone::parse("203.0.113.0/33").is_err());
        assert!(NetworkZone::parse("2001:db8::/129").is_err());
        assert!(NetworkZone::parse("not-a-cidr").is_err());
    }

    #[test]
    fn network_zone_rejects_oversized_cidr_and_clamps_error() {
        let long = "x".repeat(2048);
        let cidr = format!("{long}/24");
        let err = NetworkZone::parse(&cidr).unwrap_err();
        let IngressConfigError::InvalidZone(msg) = err else {
            panic!("expected InvalidZone error");
        };
        assert_eq!(msg.len(), MAX_INGRESS_CONFIG_ERROR_BYTES);
        assert!(msg.is_char_boundary(msg.len()));
    }

    #[test]
    fn listener_binding_rejects_empty_domain() {
        let identity = LocalIdentity {
            listener_id: "l".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            domain: "3402000000".to_string(),
            realm: "3402000000".to_string(),
        };
        let tenant = TenantId::from_uuid(uuid::Uuid::from_u128(1));
        assert!(matches!(
            ListenerBinding::new("", tenant, identity),
            Err(IngressConfigError::EmptyField("domain"))
        ));
    }

    #[test]
    fn body_identity_uses_from_user() {
        let ident = RequestIdentity {
            request_uri_host: "h".to_string(),
            to_host: "h".to_string(),
            from_user: "34020000001320000001".to_string(),
            to_user: "34020000002000000001".to_string(),
            body_device_id: Some("34020000001320000001".to_string()),
            observed_source: "203.0.113.10".parse().unwrap(),
        };
        assert!(validate_body_identity(IngressMethod::Keepalive, &ident).is_ok());
    }

    #[test]
    fn body_identity_rejects_mismatch() {
        let ident = RequestIdentity {
            request_uri_host: "h".to_string(),
            to_host: "h".to_string(),
            from_user: "34020000001320000001".to_string(),
            to_user: "34020000002000000001".to_string(),
            body_device_id: Some("34020000001320009999".to_string()),
            observed_source: "203.0.113.10".parse().unwrap(),
        };
        assert!(matches!(
            validate_body_identity(IngressMethod::Keepalive, &ident),
            Err(IngressError::BodyIdentityMismatch)
        ));
    }

    #[test]
    fn keepalive_requires_body_device_id() {
        let ident = RequestIdentity {
            request_uri_host: "h".to_string(),
            to_host: "h".to_string(),
            from_user: "34020000001320000001".to_string(),
            to_user: String::new(),
            body_device_id: None,
            observed_source: "203.0.113.10".parse().unwrap(),
        };
        assert!(matches!(
            validate_body_identity(IngressMethod::Keepalive, &ident),
            Err(IngressError::BodyIdentityMismatch)
        ));
        assert!(validate_body_identity(IngressMethod::Register, &ident).is_ok());
    }

    #[test]
    fn endpoint_update_only_on_authenticated_register() {
        assert!(
            AccessIngress::authorize_endpoint_update(IngressMethod::Register, true, false).is_ok()
        );
        assert!(
            AccessIngress::authorize_endpoint_update(IngressMethod::Register, false, true).is_ok()
        );
        assert!(matches!(
            AccessIngress::authorize_endpoint_update(IngressMethod::Register, false, false),
            Err(IngressError::AuthenticationRequired)
        ));
        assert!(matches!(
            AccessIngress::authorize_endpoint_update(IngressMethod::Keepalive, true, false),
            Err(IngressError::EndpointUpdateForbidden)
        ));
        assert!(matches!(
            AccessIngress::authorize_endpoint_update(IngressMethod::Message, true, true),
            Err(IngressError::EndpointUpdateForbidden)
        ));
    }

    #[test]
    fn sip_status_mapping_is_stable() {
        assert_eq!(IngressError::UnconfiguredDomain.sip_status(), 404);
        assert_eq!(IngressError::AmbiguousDomain.sip_status(), 403);
        assert_eq!(IngressError::BodyIdentityMismatch.sip_status(), 403);
        assert_eq!(IngressError::SourceZoneRejected.sip_status(), 403);
        assert_eq!(IngressError::AuthenticationRequired.sip_status(), 401);
        assert_eq!(IngressError::EndpointUpdateForbidden.sip_status(), 403);
        assert_eq!(
            IngressError::Session(SessionLinkError::NotRegistered).sip_status(),
            403
        );
    }
}
