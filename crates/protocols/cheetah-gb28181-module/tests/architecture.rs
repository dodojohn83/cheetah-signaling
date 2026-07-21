//! Architecture contract test for `cheetah-gb28181-module`.
//!
//! Verifies that the module layer remains Sans-I/O and does not pull in
//! runtime crates such as Tokio or the plugin SDK.

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
fn module_does_not_depend_on_tokio_or_plugin_sdk() {
    let manifest = crate_root().join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("Cargo.toml should exist");

    let deps = read_section(&text, "dependencies").unwrap_or_default();
    for forbidden in ["tokio", "cheetah-plugin-sdk", "async-trait"] {
        assert!(
            !deps.contains(forbidden),
            "module production dependencies must not include {forbidden}"
        );
    }
}

#[test]
fn module_depends_on_core() {
    let manifest = crate_root().join("Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("Cargo.toml should exist");

    let deps = read_section(&text, "dependencies").unwrap_or_default();
    assert!(
        deps.contains("cheetah-gb28181-core"),
        "module production dependencies must include cheetah-gb28181-core"
    );
}
