#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
temp_dir="$(mktemp -d)"
trap 'rm -rf "$temp_dir"' EXIT

fixture_repo="$temp_dir/fixture repo"
mkdir -p "$fixture_repo/scripts/android" "$fixture_repo/assets" \
  "$temp_dir/bin" "$temp_dir/elsewhere"
cp "$repo_root/scripts/android/saf_selftest.sh" "$fixture_repo/scripts/android/"
cp "$repo_root/assets/startup_0.ogg" "$fixture_repo/assets/"

export PATH="$temp_dir/bin:$PATH"
export EXPECTED_FIXTURE="$fixture_repo/assets/startup_0.ogg"
export PUSHED_FIXTURE="$temp_dir/pushed.ogg"
export EXPECTED_LOG="$temp_dir/expected.log"
printf 'I/hub: self-test passed\n' > "$EXPECTED_LOG"

cat > "$temp_dir/bin/adb" <<'ADB'
#!/usr/bin/env bash
set -euo pipefail

case "$*" in
  devices)
    printf 'List of devices attached\nfake-device\tdevice\n'
    ;;
  "shell mkdir -p '/sdcard/Documents/rune-selftest'" | "logcat -c")
    ;;
  push\ *)
    if [[ ! "$2" -ef "$EXPECTED_FIXTURE" ]]; then
      printf 'adb push did not find the tracked fixture from %s\n' "$PWD" >&2
      exit 1
    fi
    [[ "$3" == /sdcard/Documents/rune-selftest/startup_0.ogg ]]
    cp "$2" "$PUSHED_FIXTURE"
    ;;
  "logcat -d")
    cat "$EXPECTED_LOG"
    ;;
  *)
    printf 'Unexpected adb command: %s\n' "$*" >&2
    exit 1
    ;;
esac
ADB
chmod +x "$temp_dir/bin/adb"

check_run() (
  cd "$1"
  printf '\n' | "$2" >/dev/null

  shopt -s nullglob
  logs=("$fixture_repo/scripts/android"/selftest-log-*.txt)
  if [[ ${#logs[@]} -ne 1 || -e "$fixture_repo/scripts/scripts" ]]; then
    printf 'Expected one log in scripts/android after running from %s\n' "$PWD" >&2
    exit 1
  fi
  cmp "$EXPECTED_FIXTURE" "$PUSHED_FIXTURE"
  cmp "$EXPECTED_LOG" "${logs[0]}"
  rm "$PUSHED_FIXTURE" "${logs[0]}"
)

check_run "$fixture_repo" scripts/android/saf_selftest.sh
check_run "$temp_dir/elsewhere" "$fixture_repo/scripts/android/saf_selftest.sh"
printf 'SAF self-test fixture and log paths passed from both working directories.\n'
