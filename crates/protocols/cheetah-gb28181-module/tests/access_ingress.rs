//! Integration tests for GB28181 listener-tenant routing, body identity and
//! endpoint-security validation (`GB4-ACC-003`).
//!
//! These drive [`AccessIngress`] over the in-memory
//! [`ProtocolSessionRepository`] contract implementation and assert the
//! rejection behaviour for tenant mismatch, body-identity mismatch, endpoint
//! hijack attempts, domain ambiguity, unconfigured domains, source zones and
//! stale owners.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::IpAddr;
use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, InMemoryProtocolSessionRepository,
};
use cheetah_domain::{
    CompatibilityProfile, LocalIdentity, PresenceState, Protocol, ProtocolSessionRepository,
    RegistrationInfo, SessionEndpoint, SipTransport,
};
use cheetah_gb28181_module::{
    AccessIngress, DeviceBinding, IngressError, ListenerBinding, NetworkZone, ProtocolSessionLink,
    RegisterParams, RequestIdentity, SessionLinkError,
};
use cheetah_signal_types::{
    Clock, DeviceId, DurationMs, NodeId, OwnerEpoch, ProtocolIdentity, TenantId,
};
use futures::executor::block_on;

const EXPIRES_SECS: u32 = 3600;
const DOMAIN_A: &str = "3402000000";
const DOMAIN_B: &str = "1101000000";
const DEVICE_GB_ID: &str = "34020000001320000001";
const PLATFORM_GB_ID: &str = "34020000002000000001";

/// Test fixture. `repo` is the only field mutably borrowed during a call; the
/// link is cloned into a local so borrows stay disjoint.
struct Fixture {
    clock: Arc<InMemoryClock>,
    link: ProtocolSessionLink,
    repo: InMemoryProtocolSessionRepository,
    tenant_a: TenantId,
    tenant_b: TenantId,
    device_id: DeviceId,
    node_id: NodeId,
}

impl Fixture {
    fn new(zones: Vec<NetworkZone>) -> (Self, AccessIngress) {
        let clock = Arc::new(InMemoryClock::new());
        let id_generator = Arc::new(InMemoryIdGenerator::new());
        let link = ProtocolSessionLink::new(clock.clone(), id_generator);
        let tenant_a = TenantId::from_uuid(uuid::Uuid::from_u128(1));
        let tenant_b = TenantId::from_uuid(uuid::Uuid::from_u128(2));
        let listener_a = ListenerBinding::new(DOMAIN_A, tenant_a, local_identity(DOMAIN_A))
            .unwrap()
            .with_allowed_zones(zones);
        let listener_b =
            ListenerBinding::new(DOMAIN_B, tenant_b, local_identity(DOMAIN_B)).unwrap();
        let ingress = AccessIngress::new(vec![listener_a, listener_b]);
        (
            Self {
                clock,
                link,
                repo: InMemoryProtocolSessionRepository::new(),
                tenant_a,
                tenant_b,
                device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(10)),
                node_id: NodeId::from_uuid(uuid::Uuid::from_u128(20)),
            },
            ingress,
        )
    }

    fn binding(&self, epoch: u64) -> DeviceBinding {
        DeviceBinding {
            device_id: self.device_id,
            protocol_identity: ProtocolIdentity::new(DEVICE_GB_ID).unwrap(),
            transport: SipTransport::Udp,
            owner_node_id: self.node_id,
            owner_epoch: OwnerEpoch(epoch),
            compatibility: CompatibilityProfile::default(),
        }
    }

    fn register_params(&self, cseq: u32, source: &str) -> RegisterParams {
        let expiry_at = self
            .clock
            .now_wall()
            .checked_add(DurationMs::from_seconds(i64::from(EXPIRES_SECS)))
            .unwrap();
        RegisterParams {
            endpoint: SessionEndpoint {
                observed_source: source.to_string(),
                contact_uri: format!("sip:{DEVICE_GB_ID}@{source}"),
                advertised_endpoint: "192.0.2.1:5060".to_string(),
            },
            registration: RegistrationInfo {
                call_id: "call-id-0001".to_string(),
                cseq,
                expires_secs: EXPIRES_SECS,
            },
            expiry_at,
        }
    }

    /// Performs an authenticated REGISTER from `source`, returning the result.
    fn do_register(
        &mut self,
        ingress: &AccessIngress,
        epoch: u64,
        source: &str,
    ) -> Result<(), IngressError> {
        let link = self.link.clone();
        let binding = self.binding(epoch);
        let params = self.register_params(1, source);
        let ident = register_ident(DOMAIN_A, ip(source_ip(source)));
        block_on(ingress.register(&mut self.repo, &link, &ident, &binding, params, true))
            .map(|_| ())
    }
}

fn local_identity(domain: &str) -> LocalIdentity {
    LocalIdentity {
        listener_id: format!("listener-{domain}"),
        local_device_id: PLATFORM_GB_ID.to_string(),
        domain: domain.to_string(),
        realm: domain.to_string(),
    }
}

fn ip(addr: &str) -> IpAddr {
    addr.parse().unwrap()
}

/// Strips the `:port` suffix from a `host:port` source string.
fn source_ip(source: &str) -> &str {
    source.rsplit_once(':').map_or(source, |(host, _)| host)
}

/// A REGISTER identity for `domain` from the given source.
fn register_ident(domain: &str, source: IpAddr) -> RequestIdentity {
    RequestIdentity {
        request_uri_host: domain.to_string(),
        to_host: domain.to_string(),
        from_user: DEVICE_GB_ID.to_string(),
        to_user: DEVICE_GB_ID.to_string(),
        body_device_id: None,
        observed_source: source,
    }
}

/// A Keepalive identity for `domain` from the given source and body id.
fn keepalive_ident(domain: &str, source: IpAddr, body: &str) -> RequestIdentity {
    RequestIdentity {
        request_uri_host: domain.to_string(),
        to_host: domain.to_string(),
        from_user: DEVICE_GB_ID.to_string(),
        to_user: PLATFORM_GB_ID.to_string(),
        body_device_id: Some(body.to_string()),
        observed_source: source,
    }
}

#[test]
fn register_resolves_tenant_from_listener_domain() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    // The session is created under the listener's tenant, not any caller input.
    let stored = block_on(
        f.repo
            .get_by_device(f.tenant_a, Protocol::Gb28181, f.device_id),
    )
    .unwrap()
    .expect("session persisted under tenant A");
    assert_eq!(stored.tenant_id(), f.tenant_a);
    assert_eq!(stored.local_identity().domain, DOMAIN_A);
    // Nothing leaked into tenant B.
    assert!(
        block_on(
            f.repo
                .get_by_device(f.tenant_b, Protocol::Gb28181, f.device_id)
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn unauthenticated_register_is_rejected() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    let link = f.link.clone();
    let binding = f.binding(1);
    let params = f.register_params(1, "203.0.113.10:5060");
    let ident = register_ident(DOMAIN_A, ip("203.0.113.10"));
    let err = block_on(ingress.register(&mut f.repo, &link, &ident, &binding, params, false))
        .unwrap_err();
    assert!(matches!(err, IngressError::AuthenticationRequired));
    assert_eq!(err.sip_status(), 401);
    assert!(f.repo.is_empty(), "no session created without auth");
}

#[test]
fn unconfigured_domain_is_rejected_with_404() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    let link = f.link.clone();
    let binding = f.binding(1);
    let params = f.register_params(1, "203.0.113.10:5060");
    let ident = register_ident("9999999999", ip("203.0.113.10"));
    let err =
        block_on(ingress.register(&mut f.repo, &link, &ident, &binding, params, true)).unwrap_err();
    assert!(matches!(err, IngressError::UnconfiguredDomain));
    assert_eq!(err.sip_status(), 404);
}

#[test]
fn ambiguous_domain_config_is_rejected_with_403() {
    let clock = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let link = ProtocolSessionLink::new(clock.clone(), id_generator);
    let mut repo = InMemoryProtocolSessionRepository::new();
    let tenant_a = TenantId::from_uuid(uuid::Uuid::from_u128(1));
    let tenant_b = TenantId::from_uuid(uuid::Uuid::from_u128(2));
    // Two listeners declaring the same domain: routing is ambiguous.
    let ingress = AccessIngress::new(vec![
        ListenerBinding::new(DOMAIN_A, tenant_a, local_identity(DOMAIN_A)).unwrap(),
        ListenerBinding::new(DOMAIN_A, tenant_b, local_identity(DOMAIN_A)).unwrap(),
    ]);
    let binding = DeviceBinding {
        device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(10)),
        protocol_identity: ProtocolIdentity::new(DEVICE_GB_ID).unwrap(),
        transport: SipTransport::Udp,
        owner_node_id: NodeId::from_uuid(uuid::Uuid::from_u128(20)),
        owner_epoch: OwnerEpoch(1),
        compatibility: CompatibilityProfile::default(),
    };
    let ident = register_ident(DOMAIN_A, ip("203.0.113.10"));
    let params = RegisterParams {
        endpoint: SessionEndpoint {
            observed_source: "203.0.113.10:5060".to_string(),
            contact_uri: format!("sip:{DEVICE_GB_ID}@203.0.113.10:5060"),
            advertised_endpoint: "192.0.2.1:5060".to_string(),
        },
        registration: RegistrationInfo {
            call_id: "call".to_string(),
            cseq: 1,
            expires_secs: EXPIRES_SECS,
        },
        expiry_at: clock
            .now_wall()
            .checked_add(DurationMs::from_seconds(i64::from(EXPIRES_SECS)))
            .unwrap(),
    };
    let err =
        block_on(ingress.register(&mut repo, &link, &ident, &binding, params, true)).unwrap_err();
    assert!(matches!(err, IngressError::AmbiguousDomain));
    assert_eq!(err.sip_status(), 403);
}

#[test]
fn request_uri_and_to_domain_disagreement_is_ambiguous() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    let link = f.link.clone();
    let binding = f.binding(1);
    let params = f.register_params(1, "203.0.113.10:5060");
    let ident = RequestIdentity {
        request_uri_host: DOMAIN_A.to_string(),
        to_host: DOMAIN_B.to_string(),
        from_user: DEVICE_GB_ID.to_string(),
        to_user: DEVICE_GB_ID.to_string(),
        body_device_id: None,
        observed_source: ip("203.0.113.10"),
    };
    let err =
        block_on(ingress.register(&mut f.repo, &link, &ident, &binding, params, true)).unwrap_err();
    assert!(matches!(err, IngressError::AmbiguousDomain));
}

#[test]
fn keepalive_body_identity_mismatch_is_rejected() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(1);
    // Body reports a different device than the From user.
    let ident = keepalive_ident(DOMAIN_A, ip("203.0.113.10"), "34020000001320009999");
    let err = block_on(ingress.keepalive(&mut f.repo, &link, &ident, &binding)).unwrap_err();
    assert!(matches!(err, IngressError::BodyIdentityMismatch));
    assert_eq!(err.sip_status(), 403);
}

#[test]
fn keepalive_across_tenant_is_not_registered() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(1);
    // A keepalive routed via domain B (tenant B) finds no session there.
    let ident = keepalive_ident(DOMAIN_B, ip("203.0.113.10"), DEVICE_GB_ID);
    let err = block_on(ingress.keepalive(&mut f.repo, &link, &ident, &binding)).unwrap_err();
    assert!(matches!(
        err,
        IngressError::Session(SessionLinkError::NotRegistered)
    ));
}

#[test]
fn keepalive_from_new_source_does_not_hijack_endpoint() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(1);
    // A keepalive from a *different* source must keep the endpoint untouched.
    let ident = keepalive_ident(DOMAIN_A, ip("198.51.100.20"), DEVICE_GB_ID);
    block_on(ingress.keepalive(&mut f.repo, &link, &ident, &binding)).unwrap();

    let stored = block_on(
        f.repo
            .get_by_device(f.tenant_a, Protocol::Gb28181, f.device_id),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        stored.endpoint().observed_source,
        "203.0.113.10:5060",
        "keepalive must not rewrite the stored endpoint"
    );
    assert_eq!(stored.presence(), PresenceState::Online);
}

#[test]
fn keepalive_rejects_stale_owner_epoch() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    // Registered at epoch 5.
    f.do_register(&ingress, 5, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(4);
    // A keepalive carrying an older epoch is fenced.
    let ident = keepalive_ident(DOMAIN_A, ip("203.0.113.10"), DEVICE_GB_ID);
    let err = block_on(ingress.keepalive(&mut f.repo, &link, &ident, &binding)).unwrap_err();
    assert!(matches!(
        err,
        IngressError::Session(SessionLinkError::StaleOwner { current: 5, got: 4 })
    ));
}

#[test]
fn source_outside_allowed_zone_is_rejected() {
    let zone = NetworkZone::parse("203.0.113.0/24").unwrap();
    let (mut f, ingress) = Fixture::new(vec![zone]);

    // In-zone source is admitted.
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(1);
    // Out-of-zone source is rejected before any persistence.
    let ident = keepalive_ident(DOMAIN_A, ip("198.51.100.20"), DEVICE_GB_ID);
    let err = block_on(ingress.keepalive(&mut f.repo, &link, &ident, &binding)).unwrap_err();
    assert!(matches!(err, IngressError::SourceZoneRejected));
    assert_eq!(err.sip_status(), 403);
}

#[test]
fn unregister_removes_binding_via_ingress() {
    let (mut f, ingress) = Fixture::new(Vec::new());
    f.do_register(&ingress, 1, "203.0.113.10:5060").unwrap();

    let link = f.link.clone();
    let binding = f.binding(1);
    let ident = register_ident(DOMAIN_A, ip("203.0.113.10"));
    let removed = block_on(ingress.unregister(&mut f.repo, &link, &ident, &binding, true)).unwrap();
    assert!(removed.is_some());
    assert!(f.repo.is_empty());
}
