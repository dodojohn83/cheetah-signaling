//! Architecture contract test for `cheetah-gb28181-driver-tokio`.
//!
//! Verifies that production dependencies remain aligned with the six-layer
//! architecture: the driver may depend on `cheetah-gb28181-core` but not on
//! `cheetah-gb28181-module`.

use std::fs;
use std::path::PathBuf;

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_section(text: &str, section: &str) -> Option<String> {
    let header = format!("[{section}]");
    let start = text.find(&header)? + header.len();
    let rest = &text[start..];
    let end = rest.find("\n[").unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

#[test]
fn driver_does_not_depend_on_module_in_production() {
    let manifest = crate_root().join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("Cargo.toml should exist");

    let deps = read_section(&text, "dependencies").unwrap_or_default();
    assert!(
        !deps.contains("cheetah-gb28181-module"),
        "driver-tokio production dependencies must not include cheetah-gb28181-module"
    );
}

#[test]
fn driver_depends_on_core() {
    let manifest = crate_root().join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("Cargo.toml should exist");

    let deps = read_section(&text, "dependencies").unwrap_or_default();
    assert!(
        deps.contains("cheetah-gb28181-core"),
        "driver-tokio production dependencies must include cheetah-gb28181-core"
    );
}
