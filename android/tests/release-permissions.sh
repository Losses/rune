#!/usr/bin/env bash
set -euo pipefail

for apk in "${@:?Pass the release APKs to check}"; do
    permissions=$("${ANDROID_HOME:?}/build-tools/34.0.0/aapt2" dump permissions "$apk")
    if ! grep -Fxq "uses-permission: name='android.permission.INTERNET'" <<< "$permissions"; then
        echo "$apk: missing android.permission.INTERNET" >&2
        exit 1
    fi
done
