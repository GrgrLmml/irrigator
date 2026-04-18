#!/bin/bash
# Install/update systemd units and scripts on the Pi.
# Usage: ./deploy/install.sh [ssh-target]  (default: irrigator@irrigator)
set -euo pipefail

TARGET="${1:-irrigator@irrigator}"
DIR="$(cd "$(dirname "$0")" && pwd)"

echo "==> Copying files to $TARGET"
scp "$DIR/lte-watchdog.sh" \
    "$DIR/lte-watchdog.service" \
    "$DIR/lte-watchdog.timer" \
    "$DIR/irrigator.service" \
    "$TARGET:/tmp/"

echo "==> Installing on Pi"
ssh "$TARGET" 'set -e
    sudo install -m 755 /tmp/lte-watchdog.sh /usr/local/bin/lte-watchdog.sh
    sudo install -m 644 /tmp/lte-watchdog.service /etc/systemd/system/
    sudo install -m 644 /tmp/lte-watchdog.timer /etc/systemd/system/
    sudo install -m 644 /tmp/irrigator.service /etc/systemd/system/
    sudo mkdir -p /var/log/journal
    sudo systemd-tmpfiles --create --prefix /var/log/journal
    sudo systemctl daemon-reload
    sudo systemctl enable --now lte-watchdog.timer
    sudo systemctl restart systemd-journald
    rm /tmp/lte-watchdog.sh /tmp/lte-watchdog.service /tmp/lte-watchdog.timer /tmp/irrigator.service
'

echo "==> Done"
