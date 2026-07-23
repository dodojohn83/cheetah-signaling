//! Build script for generating tonic gRPC service bindings.
//!
//! Message types are kept in `cheetah-signal-contracts`; this crate only emits
//! the tonic client/server traits and references the message types via
//! `extern_path`.

use std::env;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let crate_dir =
        env::var("CARGO_MANIFEST_DIR").map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let crate_dir = PathBuf::from(crate_dir);
    // crates/transport/cheetah-signal-grpc -> workspace root.
    let proto_dir = crate_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("proto"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "proto directory not found"))?;

    let proto_files = [
        proto_dir.join("cheetah/common/v1/common.proto"),
        proto_dir.join("cheetah/foundation/v1/error.proto"),
        proto_dir.join("cheetah/common/v1/services.proto"),
        proto_dir.join("cheetah/device/v1/device.proto"),
        proto_dir.join("cheetah/control/v1/control.proto"),
        proto_dir.join("cheetah/media/v1/media.proto"),
        proto_dir.join("cheetah/media/v1/media_context.proto"),
        proto_dir.join("cheetah/media/v1/media_resource.proto"),
        proto_dir.join("cheetah/media/v1/media_services.proto"),
        proto_dir.join("cheetah/plugin/v1/plugin.proto"),
        proto_dir.join("cheetah/cluster/v1/cluster.proto"),
    ];

    let mut config = tonic_prost_build::configure().btree_map(".");

    // Point prost at the message types in `cheetah-signal-contracts` so this
    // crate does not regenerate them.
    let extern_prefix = "::cheetah_signal_contracts";
    for (pkg, path) in [
        (".cheetah.foundation.v1", "cheetah.foundation.v1"),
        (".cheetah.common.v1", "cheetah.common.v1"),
        (".cheetah.device.v1", "cheetah.device.v1"),
        (".cheetah.control.v1", "cheetah.control.v1"),
        (".cheetah.media.v1", "cheetah.media.v1"),
        (".cheetah.plugin.v1", "cheetah.plugin.v1"),
        (".cheetah.cluster.v1", "cheetah.cluster.v1"),
    ] {
        let rust_path = format!("{}::{}", extern_prefix, path.replace('.', "::"));
        config = config.extern_path(pkg, rust_path);
    }

    if protoc_needs_optional_experimental_flag() {
        config = config.protoc_arg("--experimental_allow_proto3_optional");
    }

    config.compile_protos(&proto_files, std::slice::from_ref(&proto_dir))?;

    println!("cargo:rerun-if-changed={}", proto_dir.display());
    Ok(())
}

fn protoc_needs_optional_experimental_flag() -> bool {
    let protoc = env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());
    let output = std::process::Command::new(&protoc)
        .arg("--version")
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version_part = stdout.split_whitespace().nth(1).unwrap_or("").trim();

    if let Some(minor) = version_part
        .split('.')
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
    {
        // libprotoc 3.12.x, 3.13.x, 3.14.x require the flag.
        version_part.starts_with("3.") && (12..=14).contains(&minor)
    } else {
        false
    }
}
