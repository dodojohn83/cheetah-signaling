//! Integration tests for the persistent GB28181 session transaction link
//! (`GB4-ACC-002`).
//!
//! These exercise the REGISTER / unregister / refresh / keepalive / expiry /
//! owner-acquisition transitions against the in-memory
//! [`ProtocolSessionRepository`] contract implementation, which mirrors the SQL
//! adapters' revision and tenant semantics.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use cheetah_domain::in_memory::{
    InMemoryClock, InMemoryIdGenerator, InMemoryProtocolSessionRepository,
};
use cheetah_domain::{
    CompatibilityProfile, DomainError, LocalIdentity, PresenceState, Protocol,
    ProtocolSessionRepository, RegistrationInfo, SessionEndpoint, SipTransport,
};
use cheetah_gb28181_module::{
    ProtocolSessionLink, RegisterOutcome, RegisterParams, SessionContext, SessionLinkError,
};
use cheetah_signal_types::{
    Clock, DeviceId, DurationMs, NodeId, OwnerEpoch, PageRequest, ProtocolIdentity, TenantId,
    UtcTimestamp,
};
use futures::executor::block_on;

const EXPIRES_SECS: u32 = 3600;

struct Harness {
    clock: Arc<InMemoryClock>,
    link: ProtocolSessionLink,
    repo: InMemoryProtocolSessionRepository,
    tenant_id: TenantId,
    device_id: DeviceId,
    node_id: NodeId,
    protocol_identity: ProtocolIdentity,
}

impl Harness {
    fn new() -> Self {
        let clock = Arc::new(InMemoryClock::new());
        let id_generator = Arc::new(InMemoryIdGenerator::new());
        let link = ProtocolSessionLink::new(clock.clone(), id_generator);
        Self {
            clock,
            link,
            repo: InMemoryProtocolSessionRepository::new(),
            tenant_id: TenantId::from_uuid(uuid::Uuid::from_u128(1)),
            device_id: DeviceId::from_uuid(uuid::Uuid::from_u128(2)),
            node_id: NodeId::from_uuid(uuid::Uuid::from_u128(3)),
            protocol_identity: ProtocolIdentity::new("34020000001320000001").unwrap(),
        }
    }

    fn context(&self, owner_epoch: u64) -> SessionContext {
        SessionContext {
            tenant_id: self.tenant_id,
            device_id: self.device_id,
            protocol_identity: self.protocol_identity.clone(),
            local_identity: LocalIdentity {
                listener_id: "listener-a".to_string(),
                local_device_id: "34020000002000000001".to_string(),
                domain: "3402000000".to_string(),
                realm: "3402000000".to_string(),
            },
            transport: SipTransport::Udp,
            owner_node_id: self.node_id,
            owner_epoch: OwnerEpoch(owner_epoch),
            compatibility: CompatibilityProfile::default(),
        }
    }

    fn expiry(&self) -> UtcTimestamp {
        self.clock
            .now_wall()
            .checked_add(DurationMs::from_seconds(i64::from(EXPIRES_SECS)))
            .unwrap()
    }

    fn register_params(&self, cseq: u32, source: &str) -> RegisterParams {
        RegisterParams {
            endpoint: SessionEndpoint {
                observed_source: source.to_string(),
                contact_uri: format!("sip:34020000001320000001@{source}"),
                advertised_endpoint: "192.0.2.1:5060".to_string(),
            },
            registration: RegistrationInfo {
                call_id: "call-id-0001".to_string(),
                cseq,
                expires_secs: EXPIRES_SECS,
            },
            expiry_at: self.expiry(),
        }
    }

    fn register(&mut self, epoch: u64, cseq: u32, source: &str) -> RegisterOutcome {
        let ctx = self.context(epoch);
        let params = self.register_params(cseq, source);
        block_on(self.link.register(&mut self.repo, &ctx, params)).unwrap()
    }
}

#[test]
fn register_creates_session_and_assigns_owner() {
    let mut h = Harness::new();
    let session_id = match h.register(1, 1, "203.0.113.10:5060") {
        RegisterOutcome::Created {
            protocol_session_id,
            owner_epoch,
        } => {
            assert_eq!(owner_epoch, OwnerEpoch(1));
            protocol_session_id
        }
        other => panic!("expected Created, got {other:?}"),
    };

    let stored = block_on(h.repo.get(h.tenant_id, session_id))
        .unwrap()
        .expect("session persisted");
    assert_eq!(stored.device_id(), h.device_id);
    assert_eq!(stored.owner_node_id(), Some(h.node_id));
    assert_eq!(stored.owner_epoch(), OwnerEpoch(1));
    assert_eq!(stored.endpoint().observed_source, "203.0.113.10:5060");
    assert_eq!(stored.expiry_at(), h.expiry());
    assert_eq!(stored.registration().cseq, 1);
}

#[test]
fn refresh_updates_expiry_endpoint_and_revision_without_new_session() {
    let mut h = Harness::new();
    let created_id = match h.register(1, 1, "203.0.113.10:5060") {
        RegisterOutcome::Created {
            protocol_session_id,
            ..
        } => protocol_session_id,
        other => panic!("expected Created, got {other:?}"),
    };

    h.clock.advance(DurationMs::from_seconds(10));
    let new_expiry = h.expiry();
    let ctx = h.context(1);
    let params = RegisterParams {
        expiry_at: new_expiry,
        ..h.register_params(2, "198.51.100.20:5060")
    };
    let refreshed = block_on(h.link.register(&mut h.repo, &ctx, params)).unwrap();
    match refreshed {
        RegisterOutcome::Refreshed {
            protocol_session_id,
            revision,
        } => {
            assert_eq!(protocol_session_id, created_id, "must reuse the session");
            assert_eq!(revision.0, 1, "refresh bumps revision once");
        }
        other => panic!("expected Refreshed, got {other:?}"),
    }
    assert_eq!(h.repo.len(), 1, "no duplicate session created");

    let stored = block_on(h.repo.get(h.tenant_id, created_id))
        .unwrap()
        .unwrap();
    assert_eq!(stored.expiry_at(), new_expiry);
    assert_eq!(stored.endpoint().observed_source, "198.51.100.20:5060");
    assert_eq!(stored.presence(), PresenceState::Online);
    assert_eq!(stored.registration().cseq, 2);
}

#[test]
fn refresh_rejects_decreasing_cseq() {
    let mut h = Harness::new();
    h.register(1, 5, "203.0.113.10:5060");

    let ctx = h.context(1);
    let params = h.register_params(4, "203.0.113.10:5060");
    let err = block_on(h.link.register(&mut h.repo, &ctx, params)).unwrap_err();
    assert!(
        matches!(
            err,
            SessionLinkError::Repository(DomainError::InvalidArgument { .. })
        ),
        "decreasing CSeq must be rejected, got {err:?}"
    );
}

#[test]
fn register_rejects_stale_owner_epoch() {
    let mut h = Harness::new();
    h.register(5, 1, "203.0.113.10:5060");

    let ctx = h.context(4);
    let params = h.register_params(2, "203.0.113.10:5060");
    let err = block_on(h.link.register(&mut h.repo, &ctx, params)).unwrap_err();
    assert!(
        matches!(err, SessionLinkError::StaleOwner { current: 5, got: 4 }),
        "stale owner REGISTER must be fenced, got {err:?}"
    );
}

#[test]
fn register_applies_owner_takeover() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    let new_node = NodeId::from_uuid(uuid::Uuid::from_u128(99));
    let mut ctx = h.context(2);
    ctx.owner_node_id = new_node;
    let params = h.register_params(2, "203.0.113.10:5060");
    block_on(h.link.register(&mut h.repo, &ctx, params)).unwrap();

    let stored = block_on(
        h.repo
            .get_by_device(h.tenant_id, Protocol::Gb28181, h.device_id),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.owner_node_id(), Some(new_node));
    assert_eq!(stored.owner_epoch(), OwnerEpoch(2));
}

#[test]
fn unregister_deletes_binding_and_is_idempotent() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    let ctx = h.context(1);
    let removed = block_on(h.link.unregister(&mut h.repo, &ctx)).unwrap();
    assert!(removed.is_some(), "first unregister removes the binding");
    assert!(h.repo.is_empty(), "session deleted");

    let again = block_on(h.link.unregister(&mut h.repo, &ctx)).unwrap();
    assert!(again.is_none(), "second unregister is a no-op");
}

#[test]
fn keepalive_without_session_is_rejected() {
    let mut h = Harness::new();
    let ctx = h.context(1);
    let err = block_on(h.link.keepalive(&mut h.repo, &ctx)).unwrap_err();
    assert!(
        matches!(err, SessionLinkError::NotRegistered),
        "keepalive with no session must be rejected, got {err:?}"
    );
}

#[test]
fn keepalive_records_presence_and_bumps_revision() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    h.clock.advance(DurationMs::from_seconds(30));
    let ctx = h.context(1);
    block_on(h.link.keepalive(&mut h.repo, &ctx)).unwrap();

    let stored = block_on(
        h.repo
            .get_by_device(h.tenant_id, Protocol::Gb28181, h.device_id),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.presence(), PresenceState::Online);
    assert!(stored.last_keepalive_at().is_some());
    assert_eq!(stored.revision().0, 1);
}

#[test]
fn keepalive_on_expired_session_requires_reregistration() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    h.clock
        .advance(DurationMs::from_seconds(i64::from(EXPIRES_SECS) + 1));
    let ctx = h.context(1);
    let err = block_on(h.link.keepalive(&mut h.repo, &ctx)).unwrap_err();
    assert!(
        matches!(err, SessionLinkError::Expired),
        "expired keepalive must be rejected, got {err:?}"
    );
}

#[test]
fn keepalive_rejects_stale_owner_epoch() {
    let mut h = Harness::new();
    h.register(5, 1, "203.0.113.10:5060");

    let ctx = h.context(4);
    let err = block_on(h.link.keepalive(&mut h.repo, &ctx)).unwrap_err();
    assert!(
        matches!(err, SessionLinkError::StaleOwner { current: 5, got: 4 }),
        "stale owner keepalive must be fenced, got {err:?}"
    );
}

#[test]
fn acquire_owner_increments_epoch_and_fences_stale() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    let new_node = NodeId::from_uuid(uuid::Uuid::from_u128(42));
    let revision = block_on(h.link.acquire_owner(
        &mut h.repo,
        h.tenant_id,
        h.device_id,
        new_node,
        OwnerEpoch(2),
    ))
    .unwrap();
    assert!(revision.is_some());

    let stored = block_on(
        h.repo
            .get_by_device(h.tenant_id, Protocol::Gb28181, h.device_id),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.owner_node_id(), Some(new_node));
    assert_eq!(stored.owner_epoch(), OwnerEpoch(2));

    // An equal or older epoch is rejected as stale.
    let err = block_on(h.link.acquire_owner(
        &mut h.repo,
        h.tenant_id,
        h.device_id,
        new_node,
        OwnerEpoch(2),
    ))
    .unwrap_err();
    assert!(
        matches!(err, SessionLinkError::StaleOwner { current: 2, got: 2 }),
        "non-increasing epoch must be rejected, got {err:?}"
    );
}

#[test]
fn acquire_owner_without_session_returns_none() {
    let mut h = Harness::new();
    let result = block_on(h.link.acquire_owner(
        &mut h.repo,
        h.tenant_id,
        h.device_id,
        h.node_id,
        OwnerEpoch(1),
    ))
    .unwrap();
    assert!(result.is_none());
}

#[test]
fn reap_expired_marks_offline_and_is_idempotent() {
    let mut h = Harness::new();
    h.register(1, 1, "203.0.113.10:5060");

    // Not yet expired: nothing reaped.
    let now = h.clock.now_wall();
    assert_eq!(
        block_on(h.link.reap_expired(&mut h.repo, now, 100, 1000)).unwrap(),
        0
    );

    h.clock
        .advance(DurationMs::from_seconds(i64::from(EXPIRES_SECS) + 1));
    let now = h.clock.now_wall();
    let reaped = block_on(h.link.reap_expired(&mut h.repo, now, 100, 1000)).unwrap();
    assert_eq!(reaped, 1);

    let stored = block_on(
        h.repo
            .get_by_device(h.tenant_id, Protocol::Gb28181, h.device_id),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stored.presence(), PresenceState::Offline);
    assert_eq!(stored.offline_reason(), Some("expired"));

    // Second sweep is idempotent: already offline, nothing to do.
    let reaped_again = block_on(h.link.reap_expired(&mut h.repo, now, 100, 1000)).unwrap();
    assert_eq!(reaped_again, 0);
}

#[test]
fn reap_expired_pages_and_skips_fresh_sessions() {
    let clock = Arc::new(InMemoryClock::new());
    let id_generator = Arc::new(InMemoryIdGenerator::new());
    let link = ProtocolSessionLink::new(clock.clone(), id_generator);
    let mut repo = InMemoryProtocolSessionRepository::new();
    let tenant_id = TenantId::from_uuid(uuid::Uuid::from_u128(7));
    let node_id = NodeId::from_uuid(uuid::Uuid::from_u128(8));

    let make_ctx = |device_id: DeviceId, identity: &str| SessionContext {
        tenant_id,
        device_id,
        protocol_identity: ProtocolIdentity::new(identity).unwrap(),
        local_identity: LocalIdentity {
            listener_id: "listener-a".to_string(),
            local_device_id: "34020000002000000001".to_string(),
            domain: "3402000000".to_string(),
            realm: "3402000000".to_string(),
        },
        transport: SipTransport::Udp,
        owner_node_id: node_id,
        owner_epoch: OwnerEpoch(1),
        compatibility: CompatibilityProfile::default(),
    };
    let make_params = |call_id: &str, expires_secs: u32| RegisterParams {
        endpoint: SessionEndpoint {
            observed_source: "203.0.113.10:5060".to_string(),
            contact_uri: "sip:x@203.0.113.10:5060".to_string(),
            advertised_endpoint: "192.0.2.1:5060".to_string(),
        },
        registration: RegistrationInfo {
            call_id: call_id.to_string(),
            cseq: 1,
            expires_secs,
        },
        expiry_at: clock
            .now_wall()
            .checked_add(DurationMs::from_seconds(i64::from(expires_secs)))
            .unwrap(),
    };

    // Five sessions that will expire.
    for n in 0..5u128 {
        let device_id = DeviceId::from_uuid(uuid::Uuid::from_u128(100 + n));
        let ctx = make_ctx(device_id, &format!("340200000013200000{n:02}"));
        block_on(link.register(&mut repo, &ctx, make_params(&format!("call-{n}"), 10))).unwrap();
    }

    // One fresh session that outlives the sweep.
    let fresh_device = DeviceId::from_uuid(uuid::Uuid::from_u128(200));
    let fresh_ctx = make_ctx(fresh_device, "34020000001320009999");
    block_on(link.register(&mut repo, &fresh_ctx, make_params("fresh", 100_000))).unwrap();

    clock.advance(DurationMs::from_seconds(20));
    let now = clock.now_wall();
    // Page size of 2 forces multiple pages across the five expired sessions.
    let reaped = block_on(link.reap_expired(&mut repo, now, 2, 1000)).unwrap();
    assert_eq!(reaped, 5, "all expired sessions reaped across pages");

    let fresh = block_on(repo.get_by_device(tenant_id, Protocol::Gb28181, fresh_device))
        .unwrap()
        .unwrap();
    assert_ne!(
        fresh.presence(),
        PresenceState::Offline,
        "fresh session left untouched"
    );

    // Confirm no expired session remains online.
    let mut cursor = None;
    let mut remaining_online = 0;
    loop {
        let mut page = PageRequest::new(10).unwrap();
        page.cursor = cursor;
        let result = block_on(repo.list_expired(now, page)).unwrap();
        for s in &result.items {
            if s.presence() != PresenceState::Offline {
                remaining_online += 1;
            }
        }
        match result.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(remaining_online, 0);
}
