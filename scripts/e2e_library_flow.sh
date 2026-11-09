#!/usr/bin/env bash
set -euo pipefail

if [[ "${E2E_LIBRARY_FLOW_GUARDED:-}" != 1 ]]; then
  export E2E_LIBRARY_FLOW_GUARDED=1
  exec timeout --signal=TERM --kill-after=10s 14400s "$0" "$@"
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
run_root=$(mktemp -d "${TMPDIR:-/tmp}/rune-e2e-library.XXXXXX")
original_home=$HOME
cargo_home=${CARGO_HOME:-$original_home/.cargo}
home="$run_root/home"
runtime="$run_root/runtime"
music_a="$run_root/Music A"
music_b="$run_root/Music B"
mkdir -p "$home" "$runtime" "$music_a" "$music_b" \
  "$run_root/config" "$run_root/data" "$run_root/cache"
chmod 700 "$runtime"
cp "$repo_root/assets/startup_0.ogg" "$music_a/startup_0.ogg"
cp "$repo_root/assets/startup_1.ogg" "$music_b/startup_1.ogg"
vorbis_tools=$(nix build --no-link --print-out-paths nixpkgs#vorbis-tools)
printf 'TITLE=Startup A\nARTIST=E2E Library\nALBUM=Startup Test\n' \
  >"$run_root/music-a.tags"
printf 'TITLE=Startup B\nARTIST=E2E Library\nALBUM=Startup Test\n' \
  >"$run_root/music-b.tags"
"$vorbis_tools/bin/vorbiscomment" -w -R -c "$run_root/music-a.tags" \
  "$music_a/startup_0.ogg"
"$vorbis_tools/bin/vorbiscomment" -w -R -c "$run_root/music-b.tags" \
  "$music_b/startup_1.ogg"

xvfb_pid=
wm_pid=
cleanup() {
  if [[ -n "$wm_pid" ]] && kill -0 "$wm_pid" 2>/dev/null; then
    kill "$wm_pid"
    wait "$wm_pid" 2>/dev/null || true
  fi
  if [[ -n "$xvfb_pid" ]] && kill -0 "$xvfb_pid" 2>/dev/null; then
    kill "$xvfb_pid"
    wait "$xvfb_pid" 2>/dev/null || true
  fi
  rm -rf "$run_root"
}
trap cleanup EXIT

# Always run on a private Xvfb display: the app must never open windows on the
# developer's desktop, even when the script is started from a desktop session.
xorg_server=$(nix eval --raw nixpkgs#xorg-server.outPath)
openbox=$(nix build --no-link --print-out-paths nixpkgs#openbox)
"$xorg_server/bin/Xvfb" -displayfd 3 -screen 0 1600x1200x24 -nolisten tcp \
  -ac 3>"$run_root/xvfb-display" >"$run_root/xvfb.log" 2>&1 &
xvfb_pid=$!
for _ in $(seq 1 100); do
  [[ -s "$run_root/xvfb-display" ]] && break
  kill -0 "$xvfb_pid" || { cat "$run_root/xvfb.log" >&2; exit 1; }
  sleep 0.1
done
if [[ ! -s "$run_root/xvfb-display" ]]; then
  printf 'Timed out waiting for the isolated Xvfb display\n' >&2
  exit 1
fi
export DISPLAY=":$(cat "$run_root/xvfb-display")"
unset WAYLAND_DISPLAY
export GTK_USE_PORTAL=0 GDK_DEBUG=no-portals
"$openbox/bin/openbox" >"$run_root/openbox.log" 2>&1 &
wm_pid=$!

export HOME="$home"
export XDG_CONFIG_HOME="$run_root/config"
export XDG_DATA_HOME="$run_root/data"
export XDG_CACHE_HOME="$run_root/cache"
export XDG_RUNTIME_DIR="$runtime"
export CARGO_HOME="$cargo_home"
export E2E_LIBRARY_A="$music_a"
export E2E_LIBRARY_B="$music_b"

started=$SECONDS
for phase in 1 2 3 4; do
  export E2E_PHASE="$phase"
  command=(dbus-run-session -- nix develop -c flutter test integration_test/e2e_library_flow_test.dart -d linux)
  printf 'E2E library flow phase %s/4\n' "$phase"
  timeout --signal=TERM --kill-after=10s 3600s "${command[@]}"

  if [[ "$phase" == 1 ]]; then
    registry=$(find "$run_root" -type f -name libraries.json -print -quit)
    if [[ -z "$registry" ]]; then
      printf 'Isolation check failed: no libraries.json appeared under %s\n' "$run_root" >&2
      exit 1
    fi
    case "$registry" in
      "$run_root"/*) printf 'Isolation verified: registry is %s\n' "$registry" ;;
      *) printf 'Isolation check failed: registry escaped temp root: %s\n' "$registry" >&2; exit 1 ;;
    esac
  fi
done
printf 'E2E library flow passed in %ss\n' "$((SECONDS - started))"
