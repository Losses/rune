#!/usr/bin/env bash

# SAF layered self-test helper (semi-automatic).
#
# Prerequisites:
#   - A debug build of rune is installed on the device (flutter install / adb install).
#   - A SAF directory has already been granted to the app at least once
#     (otherwise the SAF picker dialog must be completed interactively).
#   - adb is on PATH and the device is connected with USB debugging enabled.
#
# Usage:
#   scripts/android/saf_selftest.sh [device-target-dir]
#
# What it does:
#   1. Verifies an adb device is connected.
#   2. Pushes a real audio file (assets/startup_0.ogg) to the device
#      (default: /sdcard/Documents/rune-selftest) so the scan/playback
#      chain has real media to work with.
#   3. Clears logcat and waits for you to run the self-test in the app:
#      Settings > Library > "Run filesystem self-test" (debug builds show it
#      directly; release builds require a long-press on the build info in
#      Settings > About first).
#   4. Dumps logcat, filters the Rust log tags (hub, playback, ci.not.rune),
#      saves everything to scripts/android/selftest-log-<timestamp>.txt and
#      highlights panic/error lines.

set -euo pipefail

cd "$(dirname "$0")"
cd ..

TARGET_DIR="${1:-/sdcard/Documents/rune-selftest}"
SCRIPT_DIR="scripts/android"
TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
LOG_FILE="$SCRIPT_DIR/selftest-log-$TIMESTAMP.txt"

echo "==> Checking adb device connection..."
if ! adb devices | awk 'NR > 1 && $2 == "device"' | grep -q .; then
    echo "ERROR: no adb device in 'device' state found. Connect a device and try again."
    exit 1
fi
adb devices

echo "==> Pushing test media to $TARGET_DIR ..."
adb shell "mkdir -p '$TARGET_DIR'"
adb push assets/startup_0.ogg "$TARGET_DIR/startup_0.ogg"

echo "==> Clearing logcat..."
adb logcat -c

cat <<EOF

Now on the device:
  1. Open rune.
  2. Go to Settings > Library > "Run filesystem self-test".
  3. Pick the directory you want to diagnose (e.g. the TF card SAF root,
     or the folder containing $TARGET_DIR).
  4. Wait for all layers to finish.

Press Enter here when the self-test has finished on the device...
EOF
read -r _

echo "==> Capturing logcat to $LOG_FILE ..."
mkdir -p "$SCRIPT_DIR"
adb logcat -d > "$LOG_FILE"

echo "==> Rust-related lines (hub / playback / ci.not.rune):"
grep -E "hub|playback|ci\.not\.rune" "$LOG_FILE" | tail -n 200 || echo "(no matching lines)"

echo
echo "==> panic / error highlights:"
grep -nEi "panic|error|failed" "$LOG_FILE" | tail -n 100 || echo "(no panic/error lines found)"

echo
echo "Full log saved to: $LOG_FILE"
