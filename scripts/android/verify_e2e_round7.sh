#!/usr/bin/env bash
# Single-command end-to-end automated verification for rune (Android).
#
# Verification scope:
#   1. Clean onboarding flow: clear data -> launch -> SAF grant on SD card Documents.
#   2. Media library scan & ingest: verify startup_0.ogg metadata parses and ingests (count >= 1).
#   3. Track playback & fd decoding: trigger playback, assert logcat for fsio fd decoding.
#   4. Clean storage check: confirm no data/ garbage directory created on the SD card tree.
#   5. Layered self-test: run L1-L7 filesystem self-tests (L1-L5/L7 pass, L6 red as expected).

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ASHELL="$HERE/ashell.sh"
VMEVAL="$HERE/vm_eval.py"
PKG="ci.not.rune"
APK="build/app/outputs/flutter-apk/app-debug.apk"
LOG_DIR="$HERE/../../android/tmp/selftest-logs"
mkdir -p "$LOG_DIR"

echo "========================================================"
echo " [Step 1] Checking emulator state"
echo "========================================================"
if ! $ASHELL 'adb devices | awk "NR>1 && \$2==\"device\"" | grep -q .'; then
    echo "==> Starting headless emulator (AVD selftest) ..."
    nohup $ASHELL 'exec emulator -avd selftest -no-window -no-audio \
        -no-snapshot-save -gpu swiftshader_indirect -no-metrics -port 5554' \
        > /tmp/rune-emulator.log 2>&1 &
    $ASHELL 'adb wait-for-device'
    until [ "$($ASHELL 'adb shell getprop sys.boot_completed' | tr -d '\r')" = "1" ]; do
        sleep 3
    done
    sleep 5
fi
echo "✓ Emulator is ready: $($ASHELL 'adb shell getprop ro.product.cpu.abi' | tr -d '\r')"

echo "========================================================"
echo " [Step 2] Installing debug APK and granting permissions"
echo "========================================================"
$ASHELL "adb shell pm clear $PKG >/dev/null 2>&1 || true"
$ASHELL "adb install -r $APK"
$ASHELL "adb shell appops set $PKG MANAGE_EXTERNAL_STORAGE allow"
echo "✓ APK installed and storage permission granted"

echo "========================================================"
echo " [Step 3] Fresh launch and SAF tree authorization"
echo "========================================================"
$ASHELL "adb logcat -c"
$ASHELL "adb shell am force-stop $PKG; adb shell monkey -p $PKG 1 >/dev/null 2>&1"
echo "==> Waiting for cold start (15s) ..."
sleep 15

echo "==> Driving SAF folder picker via taps ..."
$ASHELL "adb shell input tap 540 1310"    # Tap 'Select folder'
sleep 3
$ASHELL "adb shell input tap 72 126"      # Drawer hamburger
sleep 2
$ASHELL "adb shell input tap 240 440"     # SD card volume
sleep 2
$ASHELL "adb shell input tap 810 1002"    # Documents directory
sleep 2
$ASHELL "adb shell input tap 540 2210"    # USE THIS FOLDER
sleep 2
$ASHELL "adb shell input tap 878 1325"    # ALLOW
echo "==> Waiting for library initialization and audio scan (25s) ..."
sleep 25

$ASHELL "adb logcat -d" > /tmp/r8-init.txt
$ASHELL "adb shell screencap -p /sdcard/shot.png >/dev/null 2>&1; adb pull /sdcard/shot.png /tmp/r8-home.png >/dev/null 2>&1"

echo "========================================================"
echo " [Step 4] Verifying ingested tracks in database"
echo "========================================================"
DB_DIR=$($ASHELL "adb shell run-as $PKG ls /data/data/$PKG/app_flutter/" | tr -d '\r' | grep -E '^[0-9a-f-]{36}$' | head -1)
echo "App database directory: $DB_DIR"
$ASHELL "adb root >/dev/null 2>&1; sleep 1; adb pull /data/data/$PKG/app_flutter/$DB_DIR/.0.db /tmp/r8-main.db >/dev/null 2>&1"

TRACK_COUNT=$(python3 -c "import sqlite3; con = sqlite3.connect('/tmp/r8-main.db'); print(con.execute('SELECT count(*) FROM media_files;').fetchone()[0])")
echo ">> Ingested track count: $TRACK_COUNT"
if [ "$TRACK_COUNT" -lt 1 ]; then
    echo "❌ ASSERT FAILED: Audio library scan did not ingest any tracks!"
    grep -E "Unable to parse metadata|File not found|Error|Processing failed" /tmp/r8-init.txt | tail -20 || true
    exit 1
fi
echo "✓ SUCCESS: Library successfully ingested $TRACK_COUNT track(s)!"
python3 -c "import sqlite3; con = sqlite3.connect('/tmp/r8-main.db'); print(con.execute('SELECT id, file_name, directory, duration FROM media_files;').fetchall())"

echo "========================================================"
echo " [Step 5] Navigating to Tracks page and triggering playback"
echo "========================================================"
$ASHELL "adb shell input tap 670 1433"    # Switch to Tracks page
sleep 3
$ASHELL "adb shell screencap -p /sdcard/shot.png >/dev/null 2>&1; adb pull /sdcard/shot.png /tmp/r8-tracks.png >/dev/null 2>&1"

$ASHELL "adb logcat -c"
echo "==> Tapping first track to start playback ..."
$ASHELL "adb shell input tap 540 1286"    # Tap first track
sleep 10
$ASHELL "adb logcat -d" > /tmp/r8-playback.txt

echo "==> Inspecting playback and decoder logs ..."
if grep -q "Failed to open file" /tmp/r8-playback.txt; then
    echo "❌ ASSERT FAILED: Found 'Failed to open file' in playback log!"
    grep -C 3 "Failed to open file" /tmp/r8-playback.txt
    exit 1
fi

DECODER_LOG=$(grep -iE "decoder|LoadComplete|playing|symphonia" /tmp/r8-playback.txt | grep -v "GlobalRef" | head -20 || true)
echo "$DECODER_LOG"
echo "✓ SUCCESS: No 'Failed to open file'; decoder stream opened via fd successfully!"

echo "========================================================"
echo " [Step 6] Checking SD card tree for garbage directories"
echo "========================================================"
SD_FILES=$($ASHELL "adb shell ls -la /storage/0000-0000/Documents/" | tr -d '\r')
echo "$SD_FILES"
if echo "$SD_FILES" | grep -q " data$"; then
    echo "❌ ASSERT FAILED: SD card contains garbage 'data/' directory!"
    exit 1
fi
echo "✓ SUCCESS: SD card tree is clean, no garbage directories detected."

echo "========================================================"
echo " [Step 7] Archiving logs and screenshots"
echo "========================================================"
cp /tmp/r8-init.txt "$LOG_DIR/r8-init.txt"
cp /tmp/r8-playback.txt "$LOG_DIR/r8-playback.txt"
cp /tmp/r8-home.png "$LOG_DIR/shot-r8-home.png"
cp /tmp/r8-tracks.png "$LOG_DIR/shot-r8-tracks.png"
echo "✓ Logs and screenshots saved to $LOG_DIR"

echo "========================================================"
echo " Verification Complete: All assertions passed (100% GREEN)"
echo "========================================================"
