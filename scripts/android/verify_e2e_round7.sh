#!/usr/bin/env bash
# On a disposable Android device, add an SD-card library through the system
# folder picker, verify the scan and the fixture's playback, and check storage.
# Use Rune's Android debug build environment with rinf, Flutter and adb on PATH.
# The default AVD is "selftest" (see emulator_selftest.sh). Set ANDROID_SERIAL
# for another Android 11+ test phone with a mounted FAT SD card and DocumentsUI.
# This clears Rune's app data on the selected device.
set -euo pipefail

run_playback_test() {
    local tree_id=$1 log=$2
    flutter test integration_test/android_saf_playback_test.dart \
        -d "$ANDROID_SERIAL" --dart-define="RUNE_SAF_TREE_ID=$tree_id" \
        --reporter expanded | tee "$log" || return 1

    # Only the device test can emit this after observing the fixture advance.
    if ! grep -Fq 'Verified Android SAF playback: startup_0.ogg ' "$log"; then
        echo "Missing playback evidence for startup_0.ogg" >&2
        return 1
    fi
}

verify_android() (
    local here
    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    cd "$here/../.."
    if ! command -v adb >/dev/null || ! command -v flutter >/dev/null; then
        exec "$here/ashell.sh" 'command -v adb && command -v flutter && exec scripts/android/verify_e2e_round7.sh'
    fi
    rinf gen
    flutter gen-l10n

    local log_dir
    mkdir -p android/tmp/selftest-logs
    log_dir=$(mktemp -d "$PWD/android/tmp/selftest-logs/round7.XXXXXX")
    echo "Android verification logs: $log_dir"

    if [[ -z ${ANDROID_SERIAL:-} ]]; then
        export ANDROID_SERIAL=emulator-5554
        if ! adb get-state >/dev/null 2>&1; then
            nohup emulator -avd selftest -port 5554 -no-window -no-audio \
                -no-snapshot-save -gpu swiftshader_indirect -no-metrics \
                >"$log_dir/emulator.log" 2>&1 &
        fi
        local deadline=$((SECONDS + 180))
        until [[ $(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r') == 1 ]]; do
            if (( SECONDS >= deadline )); then
                echo "Timed out booting the selftest emulator" >&2
                return 1
            fi
            sleep 1
        done
        if [[ $(adb emu avd name | tr -d '\r' | head -1) != selftest ]]; then
            echo "The default emulator port is in use by another AVD" >&2
            return 1
        fi
    fi

    local volume documents folder tree_id
    volume=$(adb shell sm list-volumes public | tr -d '\r' | awk '$2 == "mounted" { print $3 }')
    if [[ ! $volume =~ ^[[:xdigit:]]{4}-[[:xdigit:]]{4}$ ]]; then
        echo "Expected one mounted FAT SD card, found volume UUID(s): $volume" >&2
        return 1
    fi
    documents=/storage/$volume/Documents
    adb shell mkdir -p "$documents"
    folder=$(adb shell mktemp -d "$documents/rune-e2e.XXXXXX" | tr -d '\r')
    tree_id="$volume:Documents/${folder##*/}"
    [[ $folder == "$documents"/rune-e2e.?????? ]]
    adb push assets/startup_0.ogg "$folder/startup_0.ogg"

    local pkg=ci.not.rune
    if adb shell pm list packages "$pkg" | tr -d '\r' | grep -Fxq "package:$pkg"; then
        adb shell pm clear "$pkg"
    fi
    adb logcat -c

    # Grant access to the newly launched test APK. An install can replace an
    # older package and reset any app-op that was granted during the build.
    (
        until adb shell pidof "$pkg" >/dev/null 2>&1; do sleep 1; done
        adb shell appops set --uid "$pkg" MANAGE_EXTERNAL_STORAGE allow
        exec python3 "$here/confirm_saf_tree.py" "${folder##*/}"
    ) &
    local picker=$!
    # Keep logcat and a final screenshot even if a device assertion fails.
    trap 'kill "$picker" 2>/dev/null || true; wait "$picker" 2>/dev/null || true
          adb logcat -d >"$log_dir/logcat.txt" || true
          adb exec-out screencap -p >"$log_dir/screen.png" || true' EXIT

    run_playback_test "$tree_id" "$log_dir/playback.txt"

    adb shell ls -la "$folder"
    adb shell "test -f '$folder/startup_0.ogg' && test ! -e '$folder/data'" || {
        echo "Fixture folder is missing its track or contains an unexpected data/ entry" >&2
        return 1
    }
    # Local mode also writes its .rune redirect inside this disposable tree.
    adb shell rm -r "$folder"
    echo "Android scan, playback and storage checks passed."
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    verify_android
fi
