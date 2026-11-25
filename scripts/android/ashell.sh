#!/usr/bin/env bash
# Wrapper: run adb/emulator/avdmanager commands inside the nix devShell.
# Usage: scripts/android/ashell.sh adb devices
#        scripts/android/ashell.sh 'adb shell getprop sys.boot_completed'
set -euo pipefail
cd "$(dirname "$0")/../.." # repo root
export ASE_CMD="$*"
exec nix develop --command bash -c '
  setup_android_env >/dev/null 2>&1
  export PATH="$ANDROID_HOME/emulator:$ANDROID_HOME/cmdline-tools/latest/bin:$PATH"
  eval "$ASE_CMD"
'
