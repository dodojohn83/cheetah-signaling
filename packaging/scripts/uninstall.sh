#!/bin/sh
# Uninstall Cheetah Signaling binaries, systemd unit and configuration.
# Intentionally preserves /var/lib/cheetah and /var/log/cheetah.
# Usage: uninstall.sh

set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "This script must be run as root" >&2
    exit 1
fi

if command -v systemctl >/dev/null 2>&1; then
    systemctl stop cheetah-signaling.service || true
    systemctl disable cheetah-signaling.service || true
fi

rm -f /usr/bin/cheetah-signaling
rm -f /usr/lib/cheetah/cheetah-signaling-preflight
rm -f /usr/lib/systemd/system/cheetah-signaling.service
rmdir /usr/lib/cheetah 2>/dev/null || true
rm -f /etc/cheetah/config.toml
rmdir /etc/cheetah 2>/dev/null || true

if command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload
fi

echo "Cheetah Signaling uninstalled. Data in /var/lib/cheetah and /var/log/cheetah was preserved."
