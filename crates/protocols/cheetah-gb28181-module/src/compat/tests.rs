//! Unit tests for the GB28181 compatibility profile schema, validation,
//! capability negotiation, selection and revision pinning.
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

use super::*;

fn config(id: &str, standard: &str) -> CompatibilityProfileConfig {
    CompatibilityProfileConfig {
        id: id.to_string(),
        revision: 1,
        standard_version: standard.to_string(),
        manufacturer: None,
        model: None,
        firmware: None,
        evidence_ref: None,
        capabilities: Vec::new(),
        digest_algorithm: None,
        charset: None,
        rport: None,
        source_route: None,
        catalog_fragment_size: None,
        sdp_emit_ssrc: None,
        sdp_setup: None,
        sdp_send_media_before_ack: None,
        media_status_accept_121: None,
        broadcast_handshake: None,
    }
}

#[test]
fn standard_version_parses_known_tokens() {
    assert_eq!(
        StandardVersion::parse("2016"),
        Some(StandardVersion::Gb2016)
    );
    assert_eq!(
        StandardVersion::parse("GB2022"),
        Some(StandardVersion::Gb2022)
    );
    assert_eq!(StandardVersion::parse("2030"), None);
}

#[test]
fn capability_round_trips() {
    for cap in [
        CompatibilityCapability::Ipv6Transport,
        CompatibilityCapability::Broadcast,
        CompatibilityCapability::MediaStatusReport,
        CompatibilityCapability::CustomExternalId,
    ] {
        assert_eq!(CompatibilityCapability::parse(cap.as_str()), Some(cap));
    }
    assert_eq!(
        CompatibilityCapability::parse("IPv6"),
        Some(CompatibilityCapability::Ipv6Transport)
    );
    assert_eq!(CompatibilityCapability::parse("bogus"), None);
}

#[test]
fn profile_id_rejects_illegal_characters() {
    assert!(ProfileId::new("vendor-model.fw_1").is_ok());
    assert!(ProfileId::new("").is_err());
    assert!(ProfileId::new("has space").is_err());
    assert!(ProfileId::new("a".repeat(ProfileId::MAX_LEN + 1)).is_err());
}

#[test]
fn match_key_enforces_hierarchy() {
    // firmware without model is rejected.
    assert!(matches!(
        ProfileMatchKey::new(Some("m".into()), None, Some("fw".into())),
        Err(CompatibilityError::InvalidMatchKey(_))
    ));
    // model without manufacturer is rejected.
    assert!(matches!(
        ProfileMatchKey::new(None, Some("x".into()), None),
        Err(CompatibilityError::InvalidMatchKey(_))
    ));
    // full hierarchy is accepted.
    assert!(ProfileMatchKey::new(Some("m".into()), Some("x".into()), Some("fw".into())).is_ok());
    // generic is accepted.
    assert!(ProfileMatchKey::generic().specificity() == MatchSpecificity::StandardGeneric);
}

#[test]
fn from_config_builds_generic_profile_with_safe_defaults() {
    let profile = CompatibilityProfile::from_config(config("generic-2016", "2016")).unwrap();
    assert_eq!(profile.standard_version(), StandardVersion::Gb2016);
    assert_eq!(profile.digest(), DigestAlgorithmPreference::Sha256);
    assert_eq!(profile.charset(), CharsetPreference::Utf8Strict);
    assert_eq!(profile.endpoint().rport(), RportPolicy::Honor);
    assert_eq!(
        profile.endpoint().source_route(),
        SourceRoutePolicy::DialogTarget
    );
    assert_eq!(
        profile.catalog().fragment_size(),
        CatalogOverrides::DEFAULT_FRAGMENT_SIZE
    );
    // strict-default: overrides are off unless explicitly enabled.
    assert!(!profile.media_status().broadcast_handshake());
    assert!(!profile.media_status().accept_status_code_121());
    assert!(!profile.sdp().send_media_before_ack());
    assert_eq!(
        profile.match_key().specificity(),
        MatchSpecificity::StandardGeneric
    );
}

#[test]
fn from_config_rejects_unknown_tokens() {
    let mut c = config("p", "2016");
    c.standard_version = "1999".into();
    assert!(matches!(
        CompatibilityProfile::from_config(c),
        Err(CompatibilityError::UnknownStandardVersion(_))
    ));

    let mut c = config("p", "2016");
    c.charset = Some("latin1".into());
    assert!(matches!(
        CompatibilityProfile::from_config(c),
        Err(CompatibilityError::UnknownOverrideValue {
            field: "charset",
            ..
        })
    ));

    let mut c = config("p", "2016");
    c.capabilities = vec!["nonsense".into()];
    assert!(matches!(
        CompatibilityProfile::from_config(c),
        Err(CompatibilityError::UnknownCapability(_))
    ));
}

#[test]
fn catalog_fragment_size_is_bounded() {
    assert!(CatalogOverrides::new(0).is_err());
    assert!(CatalogOverrides::new(CatalogOverrides::MAX_FRAGMENT_SIZE + 1).is_err());
    assert!(CatalogOverrides::new(64).is_ok());

    let mut c = config("p", "2016");
    c.catalog_fragment_size = Some(0);
    assert!(matches!(
        CompatibilityProfile::from_config(c),
        Err(CompatibilityError::InvalidCatalogFragmentSize { .. })
    ));
}

#[test]
fn override_requires_declared_capability() {
    // broadcast_handshake without the broadcast capability is rejected.
    let mut c = config("p", "2016");
    c.broadcast_handshake = Some(true);
    assert!(CompatibilityProfile::from_config(c).is_err());

    // profile-enabled acceptance: with the capability it validates.
    let mut c = config("p", "2016");
    c.broadcast_handshake = Some(true);
    c.capabilities = vec!["broadcast".into()];
    let profile = CompatibilityProfile::from_config(c).unwrap();
    assert!(profile.media_status().broadcast_handshake());
    assert!(profile.supports(CompatibilityCapability::Broadcast));

    // media_status_accept_121 requires the media_status_report capability.
    let mut c = config("p2", "2016");
    c.media_status_accept_121 = Some(true);
    assert!(CompatibilityProfile::from_config(c).is_err());
}

#[test]
fn negotiation_is_intersection_of_declared_capabilities() {
    let mut c = config("akstream", "2016");
    c.capabilities = vec!["ipv6".into(), "broadcast".into(), "media_status".into()];
    c.broadcast_handshake = Some(true);
    c.media_status_accept_121 = Some(true);
    let profile = CompatibilityProfile::from_config(c).unwrap();

    let requested = [
        CompatibilityCapability::Ipv6Transport,
        CompatibilityCapability::PresetQuery, // not declared -> filtered out
    ];
    let effective = profile.negotiate(requested);
    assert!(effective.contains(&CompatibilityCapability::Ipv6Transport));
    assert!(!effective.contains(&CompatibilityCapability::PresetQuery));
    assert_eq!(effective.len(), 1);
}

fn built_profile(
    id: &str,
    standard: StandardVersion,
    key: ProfileMatchKey,
) -> CompatibilityProfile {
    CompatibilityProfile::builder(ProfileId::new(id).unwrap(), standard, key)
        .build()
        .unwrap()
}

#[test]
fn selection_prefers_most_specific_match() {
    let generic = built_profile(
        "generic",
        StandardVersion::Gb2016,
        ProfileMatchKey::generic(),
    );
    let by_manuf = built_profile(
        "manuf",
        StandardVersion::Gb2016,
        ProfileMatchKey::new(Some("Hikvision".into()), None, None).unwrap(),
    );
    let by_model = built_profile(
        "model",
        StandardVersion::Gb2016,
        ProfileMatchKey::new(Some("Hikvision".into()), Some("DS-2CD".into()), None).unwrap(),
    );
    let registry = CompatibilityRegistry::new([generic, by_manuf, by_model]).unwrap();

    // exact model match wins.
    let device = DeviceDescriptor::new(StandardVersion::Gb2016)
        .with_manufacturer("hikvision")
        .with_model("ds-2cd");
    match registry.select(&device).unwrap() {
        ProfileSelection::Matched {
            profile,
            specificity,
        } => {
            assert_eq!(profile.id().as_ref(), "model");
            assert_eq!(specificity, MatchSpecificity::Model);
        }
        ProfileSelection::NoMatch => panic!("expected a match"),
    }

    // unknown model falls back to manufacturer.
    let device = DeviceDescriptor::new(StandardVersion::Gb2016)
        .with_manufacturer("Hikvision")
        .with_model("other");
    match registry.select(&device).unwrap() {
        ProfileSelection::Matched { profile, .. } => assert_eq!(profile.id().as_ref(), "manuf"),
        ProfileSelection::NoMatch => panic!("expected a match"),
    }

    // unknown manufacturer falls back to generic.
    let device = DeviceDescriptor::new(StandardVersion::Gb2016).with_manufacturer("Acme");
    match registry.select(&device).unwrap() {
        ProfileSelection::Matched { profile, .. } => assert_eq!(profile.id().as_ref(), "generic"),
        ProfileSelection::NoMatch => panic!("expected a match"),
    }
}

#[test]
fn selection_no_match_for_other_standard_version() {
    let generic = built_profile("g16", StandardVersion::Gb2016, ProfileMatchKey::generic());
    let registry = CompatibilityRegistry::new([generic]).unwrap();
    let device = DeviceDescriptor::new(StandardVersion::Gb2022);
    assert_eq!(registry.select(&device).unwrap(), ProfileSelection::NoMatch);
}

#[test]
fn registry_rejects_duplicate_id() {
    let a = built_profile("dup", StandardVersion::Gb2016, ProfileMatchKey::generic());
    let b = built_profile(
        "dup",
        StandardVersion::Gb2016,
        ProfileMatchKey::new(Some("m".into()), None, None).unwrap(),
    );
    assert!(matches!(
        CompatibilityRegistry::new([a, b]),
        Err(CompatibilityError::DuplicateProfileId(_))
    ));
}

#[test]
fn registry_rejects_duplicate_match_key() {
    let a = built_profile(
        "a",
        StandardVersion::Gb2016,
        ProfileMatchKey::new(Some("Hik".into()), None, None).unwrap(),
    );
    // Same standard + same (case-insensitive) key as `a`.
    let b = built_profile(
        "b",
        StandardVersion::Gb2016,
        ProfileMatchKey::new(Some("hik".into()), None, None).unwrap(),
    );
    assert!(matches!(
        CompatibilityRegistry::new([a, b]),
        Err(CompatibilityError::DuplicateMatchKey { .. })
    ));
}

#[test]
fn revision_pinning_survives_hot_reload() {
    let key = ProfileMatchKey::new(Some("Hik".into()), None, None).unwrap();
    let profile = CompatibilityProfile::builder(
        ProfileId::new("hik").unwrap(),
        StandardVersion::Gb2016,
        key.clone(),
    )
    .revision(ProfileRevision::new(3).unwrap())
    .build()
    .unwrap();
    let pinned = profile.pin();
    assert_eq!(pinned.revision(), ProfileRevision::new(3).unwrap());

    // A newer revision supersedes the pinned snapshot but does not mutate it.
    let reloaded =
        CompatibilityProfile::builder(ProfileId::new("hik").unwrap(), StandardVersion::Gb2016, key)
            .revision(ProfileRevision::new(4).unwrap())
            .charset(CharsetPreference::GbkCompatible)
            .build()
            .unwrap();
    assert!(pinned.is_superseded_by(&reloaded));
    // the pinned snapshot keeps its original semantics.
    assert_eq!(pinned.profile().charset(), CharsetPreference::Utf8Strict);
    assert_eq!(pinned.profile().revision().get(), 3);
}

#[test]
fn revision_zero_is_rejected() {
    assert!(ProfileRevision::new(0).is_err());
    let mut c = config("p", "2016");
    c.revision = 0;
    assert!(CompatibilityProfile::from_config(c).is_err());
}
