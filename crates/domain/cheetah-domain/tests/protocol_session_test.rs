//! ProtocolSession aggregate tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{
    CompatibilityCapability, CompatibilityProfile, DomainError, LocalIdentity, NewProtocolSession,
    PresenceState, Protocol, ProtocolSession, RegistrationInfo, SessionEndpoint, SipTransport,
};
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
fn mark_offline_rejects_oversized_reason() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    let result = session.mark_offline(&clock, "x".repeat(513));
    assert!(matches!(result, Err(DomainError::InvalidArgument { .. })));
    assert_eq!(session.presence(), PresenceState::Unknown);
}

#[test]
fn mark_offline_sets_offline_reason() {
    let clock = InMemoryClock::new();
    let id_generator = InMemoryIdGenerator::new();
    let mut session = new_session(&clock, &id_generator);
    session.mark_offline(&clock, "expired").unwrap();
    assert_eq!(session.presence(), PresenceState::Offline);
    assert_eq!(session.offline_reason(), Some("expired"));
}

#[test]
fn sip_transport_from_str_is_case_insensitive_and_bounds_error() {
    assert_eq!("UDP".parse::<SipTransport>().unwrap(), SipTransport::Udp);
    assert!(matches!(
        "x".repeat(1024).parse::<SipTransport>(),
        Err(DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn presence_state_from_str_is_case_insensitive_and_bounds_error() {
    assert_eq!(
        "OFFLINE".parse::<PresenceState>().unwrap(),
        PresenceState::Offline
    );
    assert!(matches!(
        "x".repeat(1024).parse::<PresenceState>(),
        Err(DomainError::InvalidArgument { .. })
    ));
}

#[test]
fn compatibility_capability_from_str_normalizes_dash_underscore_and_bounds_error() {
    assert_eq!(
        "SDP-Media-Override"
            .parse::<CompatibilityCapability>()
            .unwrap(),
        CompatibilityCapability::SdpMediaOverride
    );
    assert_eq!(
        "sdp_media_override"
            .parse::<CompatibilityCapability>()
            .unwrap(),
        CompatibilityCapability::SdpMediaOverride
    );
    assert!(matches!(
        "x".repeat(1024).parse::<CompatibilityCapability>(),
        Err(DomainError::InvalidArgument { .. })
    ));
}
