#!/bin/sh
# Build an offline install tarball for Cheetah Signaling edge deployments.
# Usage: build-edge-package.sh <target-triple> <version>

set -eu

TARGET="${1:?target triple required}"
VERSION="${2:?version required}"

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
STAGING=$(mktemp -d)
PKG_NAME="cheetah-signaling-${VERSION}-${TARGET}"
OUT_DIR="${REPO_ROOT}/target/package"

cleanup() {
    rm -rf "$STAGING"
}
trap cleanup EXIT

cd "$REPO_ROOT"

# Build release binaries.
cargo build --release --target "$TARGET" --bin cheetah-signaling --bin cheetah-ctl

# Assemble package contents.
install -d -m 755 "$STAGING/$PKG_NAME/bin"
install -d -m 755 "$STAGING/$PKG_NAME/lib"
install -d -m 755 "$STAGING/$PKG_NAME/config"
install -d -m 755 "$STAGING/$PKG_NAME/scripts"

install -m 755 "target/${TARGET}/release/cheetah-signaling" "$STAGING/$PKG_NAME/bin/"
install -m 755 "target/${TARGET}/release/cheetah-ctl" "$STAGING/$PKG_NAME/bin/"
install -m 755 "packaging/scripts/cheetah-signaling-preflight" "$STAGING/$PKG_NAME/lib/"
install -m 644 "packaging/systemd/cheetah-signaling.service" "$STAGING/$PKG_NAME/"
install -m 644 "config.example.toml" "$STAGING/$PKG_NAME/config/"
install -m 755 "packaging/scripts/install.sh" "$STAGING/$PKG_NAME/"
install -m 755 "packaging/scripts/upgrade.sh" "$STAGING/$PKG_NAME/"
install -m 755 "packaging/scripts/uninstall.sh" "$STAGING/$PKG_NAME/"

# Generate SBOM and license summary.
install -d -m 755 "$OUT_DIR"
cargo tree --prefix none --no-dedupe --format "{p} {l}" > "$STAGING/$PKG_NAME/ThirdPartyLicenses.txt"
cargo metadata --format-version 1 --no-deps \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps({"packages":[{"name":p["name"],"version":p["version"],"license":(p.get("license") or "")} for p in d["packages"]]}, indent=2))' \
    > "$STAGING/$PKG_NAME/${PKG_NAME}.sbom.json"

# Create tarball and checksum.
install -d -m 755 "$OUT_DIR"
tar -czf "$OUT_DIR/${PKG_NAME}.tar.gz" -C "$STAGING" "$PKG_NAME"
cd "$OUT_DIR"
sha256sum "${PKG_NAME}.tar.gz" > "${PKG_NAME}.tar.gz.sha256"

echo "Package ready: $OUT_DIR/${PKG_NAME}.tar.gz"
