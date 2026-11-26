#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/verify_e2e_round7.sh"

temp=$(mktemp -d)
trap 'rm -rf "$temp"' EXIT
export ANDROID_SERIAL=emulator-5554

# Stand in for the device runner at the host boundary. Only the real device
# test constructs the evidence line, after checking player status and position.
flutter() {
    printf '%s' "$output"
    return "$runner_exit"
}
check() {
    local name=$1 expected=$2 runner_exit=$3 output=$4 actual=0
    run_playback_test '0000-0000:Documents/test' "$temp/playback.txt" \
        >"$temp/stdout" 2>&1 || actual=$?
    if [[ $actual != "$expected" ]]; then
        printf '%s: expected exit %s, got %s\n' "$name" "$expected" "$actual" >&2
        cat "$temp/stdout" >&2
        exit 1
    fi
    echo "PASS $name"
}

evidence='Verified Android SAF playback: startup_0.ogg library=fixture track=track-1 progress=0.1s -> 0.5s scanned=1'
check empty           1 0 ''
check request_failure 1 0 'RequestFailure: Unsupported(NotImplemented)'
check open_error      1 0 'Failed to open file'
check decoder_only    1 0 'decoder started'
check playing         0 0 "$evidence"
check runner_failure  1 1 "$evidence"
