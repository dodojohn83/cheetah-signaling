#!/bin/sh
# Upgrade Cheetah Signaling on an edge host while preserving user data.
# Usage: upgrade.sh [version]

set -eu

VERSION="${1:-latest}"
BACKUP_DIR="/var/backups/cheetah-signaling-${VERSION}-$(date +%Y%m%d-%H%M%S)"
DATA_DIR="/var/lib/cheetah"
CONFIG_DIR="/etc/cheetah"

log() {
    echo "cheetah-upgrade: $*"
}

if [ "$(id -u)" -ne 0 ]; then
    echo "This script must be run as root" >&2
    exit 1
fi

# Stop the service first so the data directory is quiescent before backup.
if command -v systemctl >/dev/null 2>&1; then
    systemctl stop cheetah-signaling.service || true
fi

# Back up data and config before touching binaries.
log "creating backup at $BACKUP_DIR"
install -d -m 700 "$BACKUP_DIR"
if [ -d "$DATA_DIR" ]; then
    cp -a "$DATA_DIR" "$BACKUP_DIR/data"
fi
if [ -f "$CONFIG_DIR/config.toml" ]; then
    cp -a "$CONFIG_DIR/config.toml" "$BACKUP_DIR/config.toml"
fi

# Run the install script; it will not overwrite config or data.
./install.sh "$VERSION"

# Optionally migrate data between schema versions. For v0 we keep the existing
# SQLite database and rely on the application's built-in migrations.
log "upgrade complete; data preserved in $BACKUP_DIR"

if command -v systemctl >/dev/null 2>&1; then
    systemctl start cheetah-signaling.service
fi
