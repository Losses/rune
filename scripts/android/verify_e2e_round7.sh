#!/usr/bin/env bash
# On a disposable Android device, add an SD-card library through the system
# folder picker, verify the scan and the fixture's playback, and check storage.
# Use Rune's Android debug build environment with rinf, Flutter and adb on PATH.
# The default AVD is "selftest" (see emulator_selftest.sh). Set ANDROID_SERIAL
# for another Android 11+ test phone with a mounted FAT SD card and DocumentsUI.
# Set RUNE_SAF_TEST=portable-index to scan and reopen an index in that folder.
# This clears Rune's app data on the selected device.
set -euo pipefail

run_playback_test() {
    local tree_id=$1 log_dir=$2 pid
    local log="$log_dir/playback.txt" native_log="$log_dir/native-playback.txt"
    flutter test integration_test/android_saf_playback_test.dart \
        -d "$ANDROID_SERIAL" --dart-define="RUNE_SAF_TREE_ID=$tree_id" \
        --reporter expanded 2>&1 | tee "$log" || return 1

    # Only the device test can emit this after observing the fixture advance.
    if ! grep -Fq 'Verified Android SAF playback: startup_0.ogg ' "$log"; then
        echo "Missing playback evidence for startup_0.ogg" >&2
        return 1
    fi
    # Rust load failures go to the Android playback log, even when Flutter
    # reports a passing test. Read only this run's Rune process there.
    read -r pid < "$log_dir/app.pid" || return 1
    adb logcat -d --pid="$pid" -s playback:W > "$native_log" || return 1
    if grep -E 'Failed to open file|Failed to load track|Request (LoadResolvedItemsRequest|PlayRequest) failed|RequestFailure:' "$log" "$native_log"; then
        echo "Playback failed during verification" >&2
        return 1
    fi
}

verify_android() (
    local here scenario=${RUNE_SAF_TEST:-playback}
    case $scenario in
        playback|portable-index) ;;
        *) echo "Unknown RUNE_SAF_TEST: $scenario" >&2; return 1 ;;
    esac
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
    local pkg=ci.not.rune picker=''
    volume=$(adb shell sm list-volumes public | tr -d '\r' | awk '$2 == "mounted" { print $3 }')
    if [[ ! $volume =~ ^[[:xdigit:]]{4}-[[:xdigit:]]{4}$ ]]; then
        echo "Expected one mounted FAT SD card, found volume UUID(s): $volume" >&2
        return 1
    fi
    documents=/storage/$volume/Documents
    adb shell mkdir -p "$documents"
    folder=$(adb shell mktemp -d "$documents/rune-e2e.XXXXXX" | tr -d '\r')
    [[ $folder == "$documents"/rune-e2e.?????? ]]
    package_installed() {
        adb shell pm list packages "$pkg" | tr -d '\r' | grep -Fxq "package:$pkg"
    }
    cleanup() {
        local status=$? mode
        if [[ -n $picker ]]; then
            kill "$picker" 2>/dev/null || true
            wait "$picker" 2>/dev/null || true
        fi
        # Save evidence before clearing the test's library and SAF grants.
        adb logcat -d >"$log_dir/logcat.txt" || true
        adb exec-out screencap -p >"$log_dir/screen.png" || true
        # Flutter may already have uninstalled its test APK during teardown.
        if [[ -s "$log_dir/app.pid" ]] && package_installed; then
            adb shell pm clear "$pkg" || status=1
            if [[ -s "$log_dir/storage-mode" ]]; then
                read -r mode < "$log_dir/storage-mode"
                adb shell appops set --uid "$pkg" MANAGE_EXTERNAL_STORAGE "$mode" || status=1
            fi
        fi
        adb shell rm -rf "$folder" || status=1
        if (( status == 0 )); then echo "Android $scenario checks passed."; fi
        exit "$status"
    }
    trap cleanup EXIT
    tree_id="$volume:Documents/${folder##*/}"
    adb push assets/startup_0.ogg "$folder/startup_0.ogg"

    if package_installed; then
        adb shell pm clear "$pkg"
    fi
    adb logcat -c

    # Grant access to the newly launched test APK. An install can replace an
    # older package and reset any app-op that was granted during the build.
    (
        until pid=$(adb shell pidof -s "$pkg" 2>/dev/null | tr -d '\r'); do sleep 1; done
        [[ $pid =~ ^[0-9]+$ ]]
        printf '%s\n' "$pid" > "$log_dir/app.pid"
        mode=$(adb shell appops get --uid "$pkg" MANAGE_EXTERNAL_STORAGE |
            sed -nE 's/.*MANAGE_EXTERNAL_STORAGE: ([a-z]+).*/\1/p')
        printf '%s\n' "${mode:-default}" > "$log_dir/storage-mode"
        adb shell appops set --uid "$pkg" MANAGE_EXTERNAL_STORAGE allow
        exec python3 "$here/confirm_saf_tree.py" "${folder##*/}"
    ) &
    picker=$!

    if [[ $scenario == portable-index ]]; then
        flutter test integration_test/android_portable_index_test.dart \
            -d "$ANDROID_SERIAL" --dart-define="RUNE_SAF_TREE_ID=$tree_id" \
            --reporter expanded 2>&1 | tee "$log_dir/portable-index.txt"
    else
        run_playback_test "$tree_id" "$log_dir"
    fi

    adb shell ls -la "$folder"
    adb shell "test -f '$folder/startup_0.ogg' && test ! -e '$folder/data'" || {
        echo "Fixture folder is missing its track or contains an unexpected data/ entry" >&2
        return 1
    }
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    verify_android
fi
