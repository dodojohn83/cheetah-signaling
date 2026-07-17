#!/bin/sh
# Install Cheetah Signaling on an edge host.
# Usage: install.sh [version]
# Does not delete existing data in /var/lib/cheetah.
# This script expects to run from the root of an extracted package tarball.

set -eu

VERSION="${1:-latest}"
PREFIX="${PREFIX:-/}"
BIN_DIR="${PREFIX}/usr/bin"
LIB_DIR="${PREFIX}/usr/lib/cheetah"
CONFIG_DIR="${PREFIX}/etc/cheetah"
DATA_DIR="${PREFIX}/var/lib/cheetah"
LOG_DIR="${PREFIX}/var/log/cheetah"

log() {
    echo "cheetah-install: $*"
}

if [ "$(id -u)" -ne 0 ]; then
    echo "This script must be run as root" >&2
    exit 1
fi

# Create service user.
if ! id cheetah >/dev/null 2>&1; then
    useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin cheetah
fi

# Create directories without overwriting existing data.
install -d -m 755 "$BIN_DIR"
install -d -m 750 -o cheetah -g cheetah "$DATA_DIR"
install -d -m 750 -o cheetah -g cheetah "$LOG_DIR"
install -d -m 755 "$CONFIG_DIR"
install -d -m 755 "$LIB_DIR"

# Install binary and helper scripts.
install -m 755 "bin/cheetah-signaling" "$BIN_DIR/cheetah-signaling"
install -m 755 "lib/cheetah-signaling-preflight" "$LIB_DIR/cheetah-signaling-preflight"

# Install config only if it does not already exist.
if [ ! -f "$CONFIG_DIR/config.toml" ]; then
    install -m 640 "config/config.example.toml" "$CONFIG_DIR/config.toml"
    chown root:cheetah "$CONFIG_DIR/config.toml"
fi

# Install systemd unit.
install -m 644 "cheetah-signaling.service" "${PREFIX}/usr/lib/systemd/system/cheetah-signaling.service"

# Reload systemd and enable service.
if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
    systemctl enable cheetah-signaling.service
fi

log "Cheetah Signaling ${VERSION} installed."
log "Edit ${CONFIG_DIR}/config.toml and run 'systemctl start cheetah-signaling' when ready."
