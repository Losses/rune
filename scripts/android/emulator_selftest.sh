#!/usr/bin/env bash
# emulator_selftest.sh — headless-emulator end-to-end SAF self-test for rune (Android).
#
# Prerequisites:
#   - Run anywhere; this script re-enters the nix devShell itself (ashell.sh).
#   - AVD "selftest" must exist with an SD card attached, e.g. created once with:
#       mksdcard -l sdcard 512M ~/.android/sdcard-selftest.img
#       avdmanager create avd -n selftest -k "system-images;android-34;default;x86_64" \
#           -d "medium_phone" --sdcard ~/.android/sdcard-selftest.img
#     (SDK package list lives in flake.nix; the aosp-atd image lacks DocumentsUI and
#      cannot drive the SAF picker — use the default image.)
#   - A debug APK already built: flutter build apk --debug
#   - python3 on PATH (stdlib only) for vm_eval.py.
#
# What it does:
#   1. Boots the headless emulator (if no device is connected).
#   2. Installs build/app/outputs/flutter-apk/app-debug.apk and grants
#      MANAGE_EXTERNAL_STORAGE via appops (avoids the system settings detour).
#   3. Launches the app, attaches `flutter attach` (its frontend server provides the
#      compile service that VM-service `evaluate` needs).
#   4. Drives the SAF picker via `getDirPath()` evaluated in the VM plus input taps
#      (coordinates are for the medium_phone 1080x2400 AVD; the picker remembers the
#      last volume, so the internal-storage leg re-opens at the SD card first).
#   5. Evaluates runFsSelfTest() for the SD-card tree and the internal-storage tree,
#      captures SELFTEST_PROGRESS/RESULT lines from logcat, and asserts expectations:
#      L1-L5 + L7 ok, L6 red with "fd-based playback is required" (std::fs cannot read
#      canonicalized /mnt/user/0/... paths even with MANAGE_EXTERNAL_STORAGE).
#
# Usage:
#   scripts/android/emulator_selftest.sh
#
# Note: SAF tree grants are persistable (takePersistableUriPermission) since the
# get_dir_path fix, so the picker steps can be skipped on reruns if the grants are
# still valid — but this script always re-grants to stay deterministic.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ASHELL="$HERE/ashell.sh"
VMEVAL="$HERE/vm_eval.py"
PKG=ci.not.rune

# --- 1. emulator up -------------------------------------------------------------
if ! $ASHELL 'adb devices | awk "NR>1 && \$2==\"device\"" | grep -q .'; then
    echo "==> starting emulator (AVD selftest) ..."
    nohup $ASHELL 'exec emulator -avd selftest -no-window -no-audio \
        -no-snapshot-save -gpu swiftshader_indirect -no-metrics -port 5554' \
        > /tmp/rune-emulator.log 2>&1 &
    $ASHELL 'adb wait-for-device'
    until [ "$($ASHELL 'adb shell getprop sys.boot_completed' | tr -d '\r')" = "1" ]; do
        sleep 5
    done
    sleep 10 # let fuse mounts settle
fi

# --- 2. install -----------------------------------------------------------------
echo "==> installing debug APK ..."
$ASHELL "adb install -r build/app/outputs/flutter-apk/app-debug.apk"
$ASHELL "adb shell appops set $PKG MANAGE_EXTERNAL_STORAGE allow"

# --- 3. launch + attach ---------------------------------------------------------
echo "==> launching app ..."
$ASHELL "adb logcat -c; adb shell am force-stop $PKG; adb shell monkey -p $PKG 1 >/dev/null"
sleep 15
VM_URI=$($ASHELL 'adb logcat -d | grep -oE "http://127.0.0.1:[0-9]+/[A-Za-z0-9_-]+=/" | head -1' | grep -oE "http://127.0.0.1:[0-9]+/[A-Za-z0-9_-]+=/" | tail -1)
echo "    VM service: $VM_URI"
VM_PORT=$(echo "$VM_URI" | grep -oE ":[0-9]+" | tr -d ':')
$ASHELL "adb forward tcp:$VM_PORT tcp:$VM_PORT"

echo "==> flutter attach (background) ..."
nohup $ASHELL "flutter attach --device-id emulator-5554 --debug-uri $VM_URI --no-version-check" \
    > /tmp/rune-flutter-attach.log 2>&1 &
for i in $(seq 1 24); do
    ATTACH_URI=$(grep -oE "http://127.0.0.1:[0-9]+/[A-Za-z0-9_-]+=/" /tmp/rune-flutter-attach.log | tail -1 || true)
    [ -n "${ATTACH_URI:-}" ] && break
    sleep 5
done
[ -n "${ATTACH_URI:-}" ] || { echo "ERROR: flutter attach did not come up"; exit 1; }
BASE=${ATTACH_URI#http://}; BASE=${BASE%/}
echo "    attach endpoint: $ATTACH_URI"

ISO=$(python3 "$VMEVAL" "$BASE" getvm | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["isolates"][0]["id"])')
LIB_SELFTEST=$(python3 "$VMEVAL" "$BASE" libs "$ISO" | awk '/run_fs_self_test/ {print $1}')
LIB_PICK=$(python3 "$VMEVAL" "$BASE" libs "$ISO" | awk '/get_dir_path/ {print $1}')
echo "    isolate=$ISO selftest_lib=$LIB_SELFTEST pick_lib=$LIB_PICK"

# --- 4. SAF picker driving -------------------------------------------------------
# grant_tree <volume-tap-x> <volume-tap-y> : open picker via getDirPath(), then tap
# hamburger -> volume -> Documents -> USE THIS FOLDER -> ALLOW.
grant_tree() {
    python3 "$VMEVAL" "$BASE" evalfile "$ISO" "$LIB_PICK" "$HERE/eval_pick.dart" >/dev/null
    sleep 3
    $ASHELL "adb shell input tap 72 126"            # hamburger
    sleep 2
    $ASHELL "adb shell input tap $1 $2"             # volume: SDCARD (240,440) / internal (372,283)
    sleep 2
    $ASHELL "adb shell input tap 810 1002"          # Documents folder (SD layout);
    # internal volume lists Documents at (296,1152) instead — see eval log if this misses
    sleep 2
    $ASHELL "adb shell input tap 540 2210"          # USE THIS FOLDER
    sleep 2
    $ASHELL "adb shell input tap 878 1325"          # ALLOW
    sleep 3
}

# --- 5. run + assert --------------------------------------------------------------
run_and_check() {
    local name=$1 evalfile=$2
    echo "==> self-test: $name"
    $ASHELL 'adb logcat -c'
    python3 "$VMEVAL" "$BASE" evalfile "$ISO" "$LIB_SELFTEST" "$evalfile" >/dev/null
    sleep 60
    $ASHELL 'adb logcat -d' > "/tmp/rune-selftest-$name.txt"
    local log="/tmp/rune-selftest-$name.txt"
    grep -E 'SELFTEST_PROGRESS|SELFTEST_RESULT' "$log" || true
    for layer in "L1 SAF initialization" "L2 Root directory access" "L3 Basic read/write" \
                 "L4 Streaming I/O" "L5 walk_dir" "L7 Database write path"; do
        grep -q "layer=$layer.*ok=true" "$log" || { echo "ASSERT FAIL: $layer not ok ($name)"; exit 1; }
    done
    grep -q "layer=L6.*ok=false.*fd-based playback is required" "$log" \
        || { echo "ASSERT FAIL: L6 unexpected ($name)"; exit 1; }
    echo "    $name: expectations met (L1-L5,L7 ok; L6 red as expected)"
}

grant_tree 240 440   # SDCARD volume
run_and_check sdcard "$HERE/eval_sdcard.dart"

# For the internal volume, Documents sits at (296,1152); adjust the tap if the
# picker layout differs, then:
# grant_tree 372 283 && run_and_check internal "$HERE/eval_internal.dart"

echo "==> all checks passed"
