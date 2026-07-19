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
        proto_dir.join("cheetah/common/v1/services.proto"),
        proto_dir.join("cheetah/device/v1/device.proto"),
        proto_dir.join("cheetah/control/v1/control.proto"),
        proto_dir.join("cheetah/media/v1/media.proto"),
        proto_dir.join("cheetah/media/v1/media_context.proto"),
        proto_dir.join("cheetah/media/v1/media_services.proto"),
        proto_dir.join("cheetah/plugin/v1/plugin.proto"),
        proto_dir.join("cheetah/cluster/v1/cluster.proto"),
    ];

    tonic_prost_build::configure()
        .btree_map(".")
        .compile_protos(&proto_files, std::slice::from_ref(&proto_dir))?;

    println!("cargo:rerun-if-changed={}", proto_dir.display());
    Ok(())
}
