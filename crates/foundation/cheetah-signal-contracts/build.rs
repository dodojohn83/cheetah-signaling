//! Build script for generating protobuf and gRPC bindings.

use std::env;
use std::io;
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let crate_dir =
        env::var("CARGO_MANIFEST_DIR").map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
    let crate_dir = PathBuf::from(crate_dir);
    // crates/foundation/cheetah-signal-contracts -> workspace root.
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

    let mut builder = tonic_prost_build::configure().btree_map(".");

    // Protoc versions between 3.12 and 3.14 require an experimental flag to
    // accept proto3 `optional` fields; 3.15+ enables them by default and rejects
    // the flag as unknown. Add the flag only for the narrow range that needs
    // it so the build works with both older and newer protoc installations.
    if protoc_needs_optional_experimental_flag() {
        builder = builder.protoc_arg("--experimental_allow_proto3_optional");
    }

    builder.compile_protos(&proto_files, std::slice::from_ref(&proto_dir))?;

    println!("cargo:rerun-if-changed={}", proto_dir.display());
    Ok(())
}

fn protoc_needs_optional_experimental_flag() -> bool {
    // Match `prost_build`/`tonic_prost_build` protoc resolution: the `PROTOC`
    // environment variable takes precedence, otherwise search PATH for `protoc`.
    // If we cannot determine the version, do not add the experimental flag and
    // let the normal compile step report any real incompatibility.
    let protoc = env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());
    let output = std::process::Command::new(&protoc)
        .arg("--version")
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let version = String::from_utf8_lossy(&output.stdout);
    let version = version.split_whitespace().next_back().unwrap_or("");

    let parts: Vec<u32> = version
        .split('.')
        .take(3)
        .filter_map(|s| s.parse().ok())
        .collect();

    if parts.len() < 2 {
        return false;
    }

    let major = parts[0];
    let minor = parts.get(1).copied().unwrap_or(0);

    // The flag was introduced in 3.12 and removed in 3.15.
    major == 3 && (12..15).contains(&minor)
}
