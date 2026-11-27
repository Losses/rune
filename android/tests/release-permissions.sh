#!/usr/bin/env bash
set -euo pipefail

for apk in "${@:?Pass the release APKs to check}"; do
    permissions=$("${ANDROID_HOME:?}/build-tools/34.0.0/aapt2" dump permissions "$apk")
    for permission in INTERNET FOREGROUND_SERVICE FOREGROUND_SERVICE_MEDIA_PLAYBACK; do
        if ! grep -Fxq "uses-permission: name='android.permission.$permission'" <<< "$permissions"; then
            echo "$apk: missing android.permission.$permission" >&2
            exit 1
        fi
    done
done
