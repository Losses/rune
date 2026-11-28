#!/usr/bin/env bash
# Run on a booted Android 11+ test device with DocumentsUI and a fresh Rune
# installation. Set ANDROID_SERIAL; RUNE_ANDROID_PACKAGE selects the package
# if using a checkout configured with a separate integration application ID.
set -euo pipefail
: "${ANDROID_SERIAL:?Set ANDROID_SERIAL to the Android test device}"
cd "$(dirname "$0")/../.."
pkg=${RUNE_ANDROID_PACKAGE:-ci.not.rune}
configured=$(sed -nE 's/^[[:space:]]*applicationId[[:space:]]*=[[:space:]]*"([^"]+)"[[:space:]]*$/\1/p' android/app/build.gradle)
if [[ $pkg != "$configured" ]]; then
    echo "The configured applicationId is $configured; expected $pkg." >&2
    exit 1
fi
# flutter test installs its own APK and can uninstall it at teardown. Use a
# clean package even when another Android user currently owns the device UI.
if adb shell dumpsys package "$pkg" | grep -E 'versionCode=' >/dev/null; then
    echo "Package $pkg is already installed. Select a fresh test device or application ID." >&2
    exit 1
fi
user_id=$(adb shell am get-current-user | tr -d '\r')
name=rune-saf-recovery-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')
rinf gen
flutter gen-l10n
(
    until adb shell pidof -s "$pkg" >/dev/null 2>&1; do sleep 0.5; done
    adb shell appops set --user "$user_id" --uid "$pkg" MANAGE_EXTERNAL_STORAGE allow
    exec python3 scripts/android/confirm_saf_tree.py "$name" --count 2 --package "$pkg"
) &
picker=$!
cleanup() {
    local status=$? path=/storage/emulated/$user_id/Music/$name
    kill "$picker" 2>/dev/null || true
    wait "$picker" 2>/dev/null || true
    # Dart normally removes both paths. A lost test process cannot run teardown.
    if (( status != 0 )); then
        adb shell rm -rf "$path" "$path.moved" || echo "Could not remove $path on the test device." >&2
    fi
    if adb shell pm path --user "$user_id" "$pkg" | grep '^package:' >/dev/null; then
        if ! adb shell pm uninstall --user "$user_id" "$pkg" && (( status == 0 )); then
            status=1
        fi
    fi
    exit "$status"
}
trap cleanup EXIT
mkdir -p build/android-saf-recovery
log=$(mktemp "$PWD/build/android-saf-recovery/flutter.XXXXXX")
echo "Android SAF recovery log: $log"
if flutter test integration_test/android_saf_recovery_test.dart \
    -d "$ANDROID_SERIAL" --device-user "$user_id" \
    --dart-define="RUNE_SAF_FIXTURE_NAME=$name" --reporter expanded 2>&1 | tee "$log"; then
    if ! grep -Eq 'Verified Android SAF recovery: pid=[1-9][0-9]* record=[^[:space:]]+ retained=[^[:space:]]+ added=[^[:space:]]+' "$log"; then
        echo "Missing completed device recovery evidence in $log" >&2
        exit 1
    fi
else
    results=("${PIPESTATUS[@]}")
    if (( results[0] != 0 )); then exit "${results[0]}"; fi
    exit "${results[1]}"
fi
