#!/bin/bash
# Source this file before running the isolated macOS scale-validation tools.
# It does not change HOME, move data, launch applications, or grant permissions.
if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  echo "Source this script: source scripts/macos_scale_environment.sh" >&2
  exit 1
fi

_rune_scale_setup() {
  local volume="${RUNE_SCALE_VOLUME:-}"
  local base="${RUNE_SCALE_TOOLS:-}"
  if [[ -z "$volume" || -z "$base" ]]; then
    echo 'Set RUNE_SCALE_VOLUME to the development mount and RUNE_SCALE_TOOLS to an existing tools directory on that volume.' >&2
    return 1
  fi
  volume="$(cd -P "$volume" && pwd -P)" || return 1
  local canonical volume_device base_device parent_device available system_available
  local path part current relative
  local -a targets parts
  [[ -d "$volume" && -d "$base" ]] || { echo 'Scale validation volume or tools directory missing.' >&2; return 1; }
  canonical="$(cd -P "$base" && pwd -P)" || return 1
  [[ "$canonical" == "$volume/"* ]] || { echo 'Tools directory is not on the dedicated volume.' >&2; return 1; }
  volume_device="$(df -Pk "$volume" | awk 'NR==2 {print $1}')"
  base_device="$(df -Pk "$canonical" | awk 'NR==2 {print $1}')"
  parent_device="$(df -Pk /Volumes | awk 'NR==2 {print $1}')"
  [[ -n "$volume_device" && "$volume_device" == "$base_device" && "$volume_device" != "$parent_device" ]] || { echo 'Dedicated volume is not mounted or tools directory is on another volume.' >&2; return 1; }
  available="$(df -Pk "$canonical" | awk 'NR==2 {print $4}')"
  [[ "$available" =~ ^[0-9]+$ && "$available" -ge 20971520 ]] || { echo 'Dedicated volume needs at least 20 GiB available.' >&2; return 1; }
  system_available="$(df -Pk /System/Volumes/Data | awk 'NR==2 {print $4}')"
  [[ "$system_available" =~ ^[0-9]+$ && "$system_available" -ge 2097152 ]] || { echo 'Validation requires 2 GiB of system-volume headroom for macOS-managed files; no files were removed.' >&2; return 1; }

  # Reject symlinks and nested mounts in every existing component before any
  # export or mkdir. This also rejects dangling symlinks and prevents a cache
  # path from silently escaping onto the system volume.
  targets=(pub-cache cargo cargo-target tmp gems gem-spec-cache cocoapods-home cocoapods-cache
           Gemfile bundle bundle-user bundle-user/cache xdg-cache config clang-module-cache swift-module-cache)
  for relative in "${targets[@]}"; do
    current="$canonical"
    IFS=/ read -r -a parts <<< "$relative"
    for part in "${parts[@]}"; do
      current="$current/$part"
      [[ ! -L "$current" ]] || { echo "Symlink in validation path: $current" >&2; return 1; }
      if [[ -e "$current" ]]; then
        path="$(df -Pk "$current" | awk 'NR==2 {print $1}')"
        [[ "$path" == "$volume_device" ]] || { echo "Validation path is on another volume: $current" >&2; return 1; }
      fi
    done
  done

  export PUB_CACHE="$canonical/pub-cache" CARGO_HOME="$canonical/cargo"
  export CARGO_TARGET_DIR="$canonical/cargo-target" TMPDIR="$canonical/tmp"
  export GEM_HOME="$canonical/gems" GEM_PATH="$canonical/gems" GEM_SPEC_CACHE="$canonical/gem-spec-cache"
  export CP_HOME_DIR="$canonical/cocoapods-home" CP_CACHE_DIR="$canonical/cocoapods-cache"
  export BUNDLE_GEMFILE="$canonical/Gemfile" BUNDLE_PATH="$canonical/bundle"
  export BUNDLE_USER_HOME="$canonical/bundle-user" BUNDLE_USER_CACHE="$canonical/bundle-user/cache"
  export XDG_CACHE_HOME="$canonical/xdg-cache" XDG_CONFIG_HOME="$canonical/config"
  export CLANG_MODULE_CACHE_PATH="$canonical/clang-module-cache" SWIFT_MODULECACHE_PATH="$canonical/swift-module-cache"
  export COCOAPODS_DISABLE_STATS=true BUNDLE_FORCE_RUBY_PLATFORM=true FLUTTER_SUPPRESS_ANALYTICS=true LANG=en_US.UTF-8
  export PATH="$canonical/bin:$canonical/flutter/bin:$HOME/.cargo/bin:$PATH"
  mkdir -p "$PUB_CACHE" "$CARGO_HOME" "$CARGO_TARGET_DIR" "$TMPDIR" "$GEM_HOME" "$GEM_SPEC_CACHE" "$CP_HOME_DIR" "$CP_CACHE_DIR" "$BUNDLE_PATH" "$BUNDLE_USER_CACHE" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$CLANG_MODULE_CACHE_PATH" "$SWIFT_MODULECACHE_PATH" || return 1
  # Xcode's derived data is selected by the build invocation, not these variables.
  # Flutter's macOS build normally places it under the snapshot's build/macos;
  # verify that path and Xcode's actual invocation before building.
  # macOS sandbox containers, TCC, and OS-managed caches remain system-managed.
  printf 'Scale validation environment: %s (%s KiB available)\n' "$canonical" "$available"
}
_rune_scale_setup
_rune_scale_status=$?
unset -f _rune_scale_setup
return "$_rune_scale_status"
