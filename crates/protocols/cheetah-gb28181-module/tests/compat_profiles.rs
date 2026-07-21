//! Provenance regression tests for the compatibility profile fixtures under
//! `testdata/gb28181/compat/`.
//!
//! Each `<name>.profile.toml` is a flat `CompatibilityProfileConfig` document
//! and must deserialize and validate into a `CompatibilityProfile`, with its
//! `evidence_ref` pointing at an existing `<name>.profile.meta.toml` file that
//! carries the required provenance metadata (see
//! `dev-docs/004_gb28181-improve/90_reference_provenance_and_license.md`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use cheetah_gb28181_module::{
    CompatibilityCapability, CompatibilityProfile, CompatibilityProfileConfig,
    CompatibilityRegistry, DeviceDescriptor, ProfileSelection, StandardVersion,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../")
        .canonicalize()
        .expect("repo root")
}

fn load_profile(name: &str) -> CompatibilityProfile {
    let path = repo_root()
        .join("testdata/gb28181/compat")
        .join(format!("{name}.profile.toml"));
    let text = fs::read_to_string(&path).expect("profile fixture should exist");
    let config: CompatibilityProfileConfig =
        toml::from_str(&text).expect("profile fixture should deserialize");
    CompatibilityProfile::from_config(config).expect("profile fixture should validate")
}

fn assert_meta_present(profile: &CompatibilityProfile) {
    let evidence = profile
        .evidence_ref()
        .expect("fixture profiles must reference provenance metadata");
    let meta_path = repo_root().join(evidence);
    let meta = fs::read_to_string(&meta_path).expect("evidence metadata file should exist");
    for field in [
        "source",
        "standard",
        "expected",
        "desensitization",
        "license",
    ] {
        assert!(
            meta.contains(field),
            "metadata {evidence} must contain `{field}`"
        );
    }
}

#[test]
fn akstream_profile_loads_with_expected_capabilities() {
    let profile = load_profile("akstream-gb2016");
    assert_eq!(profile.standard_version(), StandardVersion::Gb2016);
    assert!(profile.supports(CompatibilityCapability::Broadcast));
    assert!(profile.supports(CompatibilityCapability::MediaStatusReport));
    assert!(profile.media_status().broadcast_handshake());
    assert!(profile.media_status().accept_status_code_121());
    assert_eq!(profile.catalog().fragment_size(), 64);
    assert_meta_present(&profile);
}

#[test]
fn wvp_profile_loads_with_expected_capabilities() {
    let profile = load_profile("wvp-gb2016");
    assert!(profile.supports(CompatibilityCapability::CatalogNotify));
    assert!(profile.supports(CompatibilityCapability::MultipleUpstream));
    assert!(profile.supports(CompatibilityCapability::VirtualDirectory));
    assert_meta_present(&profile);
}

#[test]
fn fixtures_form_a_registry_and_select_by_manufacturer() {
    let registry =
        CompatibilityRegistry::new([load_profile("akstream-gb2016"), load_profile("wvp-gb2016")])
            .expect("fixture registry should build");

    let device = DeviceDescriptor::new(StandardVersion::Gb2016).with_manufacturer("akstream");
    match registry.select(&device).expect("selection should succeed") {
        ProfileSelection::Matched { profile, .. } => {
            assert_eq!(profile.id().as_ref(), "akstream-gb2016-generic");
        }
        ProfileSelection::NoMatch => panic!("expected the AKStream profile to match"),
    }
}
