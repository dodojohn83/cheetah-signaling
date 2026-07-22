//! Tests for the multi-link cascade manager (`GB4-CAS-006`).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use cheetah_domain::in_memory::{InMemoryClock, InMemoryIdGenerator};
use cheetah_domain::{
    BackoffPolicy, NewPlatformLink, PlatformAcl, PlatformCredential, PlatformEndpoint,
    PlatformIdentityPair, SipTransport, SubscriptionLimits,
};
use cheetah_signal_types::{Clock, IdGenerator, ProtocolIdentity};
use secrecy::SecretString;

const LOCAL: &str = "34020000002000000001";

fn provider() -> impl CascadeCredentialProvider + Clone {
    |_: &str| Some(SecretString::from("password".to_string()))
}

#[allow(clippy::too_many_arguments)]
fn make_link(
    clock: &dyn Clock,
    ids: &dyn IdGenerator,
    direction: PlatformDirection,
    remote: &str,
    acl: PlatformAcl,
) -> GbPlatformLink {
    GbPlatformLink::new(
        clock,
        NewPlatformLink {
            platform_link_id: ids.generate_platform_link_id(),
            tenant_id: ids.generate_tenant_id(),
            direction,
            identity: PlatformIdentityPair {
                local: ProtocolIdentity::new(LOCAL).unwrap(),
                remote: ProtocolIdentity::new(remote).unwrap(),
            },
            endpoint: PlatformEndpoint {
                host: "93.184.216.34".to_string(),
                port: 5060,
                transport: SipTransport::Udp,
                realm: "1100000000".to_string(),
                domain: "1100000000".to_string(),
            },
            credential: PlatformCredential {
                credential_ref: "secret://upstream".to_string(),
                allow_md5: false,
            },
            acl,
            backoff: BackoffPolicy::default(),
            subscription_limits: SubscriptionLimits::default(),
            register_interval_secs: 3600,
            compatibility_profile_id: None,
            compatibility_profile_revision: 0,
        },
    )
    .unwrap()
}

fn permissive_acl() -> PlatformAcl {
    PlatformAcl {
        allowed_catalog_prefixes: vec!["3402000000".to_string()],
        allow_control: true,
        allow_media: true,
        denied_platform_ids: vec![],
    }
}

#[test]
fn registers_multiple_independent_upstreams() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let a = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    let b = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "12000000002000000001",
        permissive_acl(),
    );
    mgr.add_upstream(&a, provider()).unwrap();
    mgr.add_upstream(&b, provider()).unwrap();
    assert_eq!(mgr.upstream_count(), 2);

    // Registering both links produces an independent REGISTER per link.
    let out_a = mgr
        .process(
            a.platform_link_id(),
            CascadeInput {
                now: 0,
                event: CascadeEvent::Register,
            },
        )
        .unwrap();
    let out_b = mgr
        .process(
            b.platform_link_id(),
            CascadeInput {
                now: 0,
                event: CascadeEvent::Register,
            },
        )
        .unwrap();
    assert_eq!(out_a.len(), 1);
    assert_eq!(out_b.len(), 1);
}

#[test]
fn rejects_duplicate_remote_and_self_loop() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let a = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    mgr.add_upstream(&a, provider()).unwrap();

    let dup = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    assert_eq!(
        mgr.add_upstream(&dup, provider()),
        Err(CascadeRoutingError::DuplicateRemote)
    );

    // A downstream link cannot be added as an upstream.
    let down = make_link(
        &clock,
        &ids,
        PlatformDirection::Downstream,
        "13000000002000000001",
        permissive_acl(),
    );
    assert_eq!(
        mgr.add_upstream(&down, provider()),
        Err(CascadeRoutingError::WrongDirection {
            expected: "upstream"
        })
    );
}

#[test]
fn unknown_link_is_isolated_from_others() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);
    let a = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    mgr.add_upstream(&a, provider()).unwrap();

    let missing = ids.generate_platform_link_id();
    assert!(matches!(
        mgr.process(
            missing,
            CascadeInput {
                now: 0,
                event: CascadeEvent::Register,
            },
        ),
        Err(CascadeRoutingError::UnknownLink)
    ));

    // The existing link is still usable; the failed lookup did not disturb it.
    let results = mgr.tick_all(1);
    assert_eq!(results.len(), 1);
    assert!(results[0].1.is_ok());
}

#[test]
fn enrolls_downstream_and_validates_identity() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let up = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    let down = make_link(
        &clock,
        &ids,
        PlatformDirection::Downstream,
        "13000000002000000001",
        permissive_acl(),
    );
    mgr.add_upstream(&up, provider()).unwrap();
    mgr.enroll_downstream(&down).unwrap();
    assert_eq!(mgr.downstream_count(), 1);

    assert_eq!(
        mgr.validate_platform_identity("11000000002000000001")
            .unwrap(),
        up.platform_link_id()
    );
    assert_eq!(
        mgr.validate_platform_identity("13000000002000000001")
            .unwrap(),
        down.platform_link_id()
    );
    // A device-like identity that matches no enrolled platform is rejected.
    assert_eq!(
        mgr.validate_platform_identity("34020000001320000001"),
        Err(CascadeRoutingError::IdentityMismatch)
    );
}

#[test]
fn acl_gates_catalog_control_and_media() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let restricted = PlatformAcl {
        allowed_catalog_prefixes: vec!["3402000000".to_string()],
        allow_control: false,
        allow_media: false,
        denied_platform_ids: vec![],
    };
    let link = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        restricted,
    );
    let id = link.platform_link_id();
    mgr.add_upstream(&link, provider()).unwrap();

    assert!(mgr.may_share_catalog(id, "34020000001320000001").unwrap());
    assert!(!mgr.may_share_catalog(id, "99990000001320000001").unwrap());
    assert!(!mgr.may_control(id).unwrap());
    assert!(!mgr.may_bridge(id).unwrap());
}

#[test]
fn bridge_routing_detects_loops_and_hop_limits() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let link = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    let id = link.platform_link_id();
    mgr.add_upstream(&link, provider()).unwrap();

    // A fresh path is fine.
    mgr.authorize_bridge(id, &["55550000002000000001"]).unwrap();

    // Revisiting the target platform is a loop.
    assert_eq!(
        mgr.authorize_bridge(id, &["11000000002000000001"]),
        Err(CascadeRoutingError::LoopDetected)
    );

    // Revisiting the local platform is a loop.
    assert_eq!(
        mgr.authorize_bridge(id, &[LOCAL]),
        Err(CascadeRoutingError::LoopDetected)
    );

    // Exceeding the hop limit is rejected.
    let deep: Vec<String> = (0..MAX_CASCADE_HOPS).map(|i| format!("p{i}")).collect();
    let refs: Vec<&str> = deep.iter().map(String::as_str).collect();
    assert_eq!(
        mgr.authorize_bridge(id, &refs),
        Err(CascadeRoutingError::HopLimitExceeded)
    );
}

#[test]
fn bridge_denied_when_media_acl_off() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let acl = PlatformAcl {
        allowed_catalog_prefixes: vec![],
        allow_control: true,
        allow_media: false,
        denied_platform_ids: vec![],
    };
    let link = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        acl,
    );
    let id = link.platform_link_id();
    mgr.add_upstream(&link, provider()).unwrap();
    assert_eq!(
        mgr.authorize_bridge(id, &[]),
        Err(CascadeRoutingError::AclDenied)
    );
}

#[test]
fn control_ownership_is_unique_and_idempotent() {
    let clock = InMemoryClock::new();
    let ids = InMemoryIdGenerator::new();
    let mut mgr = CascadeManager::new(LOCAL);

    let a = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "11000000002000000001",
        permissive_acl(),
    );
    let b = make_link(
        &clock,
        &ids,
        PlatformDirection::Upstream,
        "12000000002000000001",
        permissive_acl(),
    );
    mgr.add_upstream(&a, provider()).unwrap();
    mgr.add_upstream(&b, provider()).unwrap();

    mgr.acquire_control("34020000001320000001", a.platform_link_id())
        .unwrap();
    // Same link re-acquiring is idempotent.
    mgr.acquire_control("34020000001320000001", a.platform_link_id())
        .unwrap();
    // A different link is refused.
    assert_eq!(
        mgr.acquire_control("34020000001320000001", b.platform_link_id()),
        Err(CascadeRoutingError::ControlConflict)
    );
    // After release, the other link may take control.
    mgr.release_control("34020000001320000001", a.platform_link_id());
    mgr.acquire_control("34020000001320000001", b.platform_link_id())
        .unwrap();

    // Removing a link frees the control it owned.
    mgr.remove_upstream(b.platform_link_id());
    mgr.acquire_control("34020000001320000001", a.platform_link_id())
        .unwrap();
}
