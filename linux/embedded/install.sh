#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="/opt/rune-embedded"
SERVICE_NAME="rune-kiosk.service"

echo "=== Installing Rune Embedded Kiosk ==="

if [ "$EUID" -ne 0 ]; then
    echo "Error: Please run as root (e.g. sudo ./install.sh)" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Target user determination (prefer current SUDO_USER, then dietpi, then pi, then first UID 1000)
TARGET_USER="${SUDO_USER:-}"
if [ -z "$TARGET_USER" ] || [ "$TARGET_USER" = "root" ]; then
    if id -u dietpi >/dev/null 2>&1; then
        TARGET_USER="dietpi"
    elif id -u pi >/dev/null 2>&1; then
        TARGET_USER="pi"
    else
        TARGET_USER="$(awk -F: '$3 == 1000 {print $1}' /etc/passwd || echo "root")"
    fi
fi
TARGET_GROUP="$(id -gn "$TARGET_USER")"
TARGET_HOME="$(eval echo "~$TARGET_USER")"

echo "Deploying for user: $TARGET_USER ($TARGET_GROUP) with HOME: $TARGET_HOME"

# Stop running service if active
systemctl stop "$SERVICE_NAME" 2>/dev/null || true

# Copy application files
mkdir -p "$INSTALL_DIR"
cp -r "$SCRIPT_DIR/bin" "$INSTALL_DIR/"
cp -r "$SCRIPT_DIR/lib" "$INSTALL_DIR/"
cp -r "$SCRIPT_DIR/data" "$INSTALL_DIR/"
chmod +x "$INSTALL_DIR/bin/flutter-pi"

# Install systemd service
sed -e "s|User=dietpi|User=$TARGET_USER|g" \
    -e "s|Group=dietpi|Group=$TARGET_GROUP|g" \
    -e "s|/home/dietpi|$TARGET_HOME|g" \
    "$SCRIPT_DIR/$SERVICE_NAME" > "/etc/systemd/system/$SERVICE_NAME"

systemctl daemon-reload

# Grant user necessary permissions (video, input, render, audio)
usermod -aG video,render,input,audio "$TARGET_USER" || true

# Setup default music directory if not present
mkdir -p "$TARGET_HOME/Music"
chown -R "$TARGET_USER:$TARGET_GROUP" "$TARGET_HOME/Music"
chown -R "$TARGET_USER:$TARGET_GROUP" "$INSTALL_DIR"

echo "=== Installation complete ==="
echo "To start Rune Kiosk: sudo systemctl start $SERVICE_NAME"
echo "To stop and return to desktop: sudo systemctl stop $SERVICE_NAME"
echo "To enable on boot: sudo systemctl enable $SERVICE_NAME"
