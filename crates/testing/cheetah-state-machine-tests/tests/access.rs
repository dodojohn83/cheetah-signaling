//! GB4-TST-002 access transition table: register / unregister / owner and the
//! GB28181 ingress endpoint-update authorization rules.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{
    CompatibilityProfile, LocalIdentity, NewProtocolSession, PresenceState, Protocol,
    ProtocolSession, RegistrationInfo, SessionEndpoint, SipTransport,
};
use cheetah_gb28181_module::{AccessIngress, IngressError, IngressMethod};
use cheetah_signal_types::{
    Clock, DurationMs, IdGenerator, OwnerEpoch, ProtocolIdentity, UtcTimestamp,
};

fn expiry(clock: &InMemoryClock, secs: i64) -> UtcTimestamp {
    clock
        .now_wall()
        .checked_add(DurationMs::from_millis(secs * 1000))
        .expect("expiry must not overflow")
}

fn new_session(clock: &InMemoryClock, id_generator: &InMemoryIdGenerator) -> ProtocolSession {
    let tenant_id = id_generator.generate_tenant_id();
    let device_id = id_generator.generate_device_id();
    let params = NewProtocolSession {
        protocol_session_id: id_generator.generate_protocol_session_id(),
        tenant_id,
        device_id,
        protocol: Protocol::Gb28181,
        protocol_identity: ProtocolIdentity::new("34020000001320000001").unwrap(),
        local_identity: LocalIdentity {
            listener_id: "listener-1".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            domain: "example.com".to_string(),
            realm: "example.com".to_string(),
        },
        transport: SipTransport::Udp,
        endpoint: SessionEndpoint {
            observed_source: "192.0.2.10:5060".to_string(),
            contact_uri: "sip:34020000001320000001@192.0.2.10:5060".to_string(),
            advertised_endpoint: "sip:34020000002000000001@example.com".to_string(),
        },
        registration: RegistrationInfo {
            call_id: "call-1".to_string(),
            cseq: 1,
            expires_secs: 3600,
        },
        expiry_at: expiry(clock, 3600),
        owner_node_id: None,
        owner_epoch: OwnerEpoch::default(),
        compatibility: CompatibilityProfile::default(),
    };
    ProtocolSession::new(clock, params).expect("session creation should succeed")
}

#[test]
fn register_starts_with_unknown_presence_and_zero_revision() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let session = new_session(&clock, &id_generator);
    assert_eq!(session.presence(), PresenceState::Unknown);
    assert_eq!(session.revision().0, 0);
    assert_eq!(session.registration().cseq, 1);
    assert!(session.owner_node_id().is_none());
}

#[test]
fn refresh_registration_marks_online_and_advances_cseq() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    clock.advance(DurationMs::from_millis(1000));
    session
        .refresh_registration(
            &clock,
            RegistrationInfo {
                call_id: "call-1".to_string(),
                cseq: 2,
                expires_secs: 3600,
            },
            expiry(&clock, 3600),
            None,
        )
        .expect("authenticated refresh should succeed");
    assert_eq!(session.presence(), PresenceState::Online);
    assert_eq!(session.registration().cseq, 2);
    assert_eq!(session.revision().0, 1);
}

#[test]
fn refresh_registration_rejects_decreasing_cseq() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    let err = session
        .refresh_registration(
            &clock,
            RegistrationInfo {
                call_id: "call-1".to_string(),
                cseq: 0,
                expires_secs: 3600,
            },
            expiry(&clock, 3600),
            None,
        )
        .expect_err("decreasing CSeq must be rejected");
    assert!(matches!(
        err,
        cheetah_domain::DomainError::InvalidArgument { .. }
    ));
    // The rejected transition must not advance the aggregate.
    assert_eq!(session.registration().cseq, 1);
    assert_eq!(session.revision().0, 0);
}

#[test]
fn keepalive_keeps_device_online_without_endpoint_change() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    let endpoint_before = session.endpoint().clone();
    session.record_keepalive(&clock);
    assert_eq!(session.presence(), PresenceState::Online);
    assert_eq!(session.endpoint(), &endpoint_before);
    assert!(session.last_keepalive_at().is_some());
}

#[test]
fn unregister_marks_offline_with_reason() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    session.record_keepalive(&clock);
    session.mark_offline(&clock, "expired");
    assert_eq!(session.presence(), PresenceState::Offline);
    assert_eq!(session.offline_reason(), Some("expired"));
}

#[test]
fn owner_assignment_fences_with_epoch() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    let node_a = id_generator.generate_node_id();
    session.assign_owner(&clock, node_a, OwnerEpoch(1));
    assert_eq!(session.owner_node_id(), Some(node_a));
    assert_eq!(session.owner_epoch(), OwnerEpoch(1));

    // A takeover by another node must carry a strictly higher epoch.
    let node_b = id_generator.generate_node_id();
    session.assign_owner(&clock, node_b, OwnerEpoch(2));
    assert_eq!(session.owner_node_id(), Some(node_b));
    assert!(session.owner_epoch().0 > OwnerEpoch(1).0);
}

#[test]
fn registration_expiry_uses_absolute_time() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let session = new_session(&clock, &id_generator);
    assert!(!session.is_expired(clock.now_wall()));
    let later = clock
        .now_wall()
        .checked_add(DurationMs::from_millis(3601 * 1000))
        .unwrap();
    assert!(session.is_expired(later));
}

/// The endpoint-update authorization matrix from `ingress.rs`: only an
/// authenticated REGISTER or an in-dialog refresh may rewrite the route.
#[test]
fn endpoint_update_authorization_matrix() {
    struct Case {
        method: IngressMethod,
        authenticated: bool,
        in_dialog_refresh: bool,
        allowed: bool,
    }

    let cases = [
        Case {
            method: IngressMethod::Register,
            authenticated: true,
            in_dialog_refresh: false,
            allowed: true,
        },
        Case {
            method: IngressMethod::Register,
            authenticated: false,
            in_dialog_refresh: true,
            allowed: true,
        },
        Case {
            method: IngressMethod::Register,
            authenticated: false,
            in_dialog_refresh: false,
            allowed: false,
        },
        Case {
            method: IngressMethod::Keepalive,
            authenticated: true,
            in_dialog_refresh: true,
            allowed: false,
        },
        Case {
            method: IngressMethod::Message,
            authenticated: true,
            in_dialog_refresh: true,
            allowed: false,
        },
    ];

    for case in cases {
        let result = AccessIngress::authorize_endpoint_update(
            case.method,
            case.authenticated,
            case.in_dialog_refresh,
        );
        assert_eq!(
            result.is_ok(),
            case.allowed,
            "method={:?} authenticated={} in_dialog={} expected allowed={}",
            case.method,
            case.authenticated,
            case.in_dialog_refresh,
            case.allowed,
        );
        if !case.allowed {
            let err = result.unwrap_err();
            assert!(matches!(
                err,
                IngressError::AuthenticationRequired | IngressError::EndpointUpdateForbidden
            ));
        }
    }
}
