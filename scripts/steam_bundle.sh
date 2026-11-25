#!/usr/bin/env bash
set -euo pipefail

# Packages newly installed GTK3/AppIndicator runtime libraries, compiled
# GSettings schemas, and Steam launch scripts into a Flutter Linux release
# bundle targeting Steam Linux Runtime 4 (steamrt4), then verifies the bundle.

usage() {
    cat <<'EOF'
Usage: ./scripts/steam_bundle.sh --bundle-dir <path> --base-packages <path> [--expected-arch <x64|arm64>]

Options:
  --bundle-dir <path>       Path to Flutter release bundle (e.g. build/linux/x64/release/bundle)
  --base-packages <path>    File listing base container packages (one per line) before apt-get install
  --expected-arch <arch>    Expected ELF target architecture (x64 or arm64; defaults from uname -m)
  --help                    Show this help message and exit
EOF
}

log() {
    echo "[steam_bundle] $*"
}

die() {
    echo "[steam_bundle] Error: $*" >&2
    exit 1
}

BUNDLE_DIR=""
BASE_PACKAGES=""
EXPECTED_ARCH=""

parse_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --bundle-dir)
                [ "$#" -ge 2 ] || die "Missing value for --bundle-dir"
                BUNDLE_DIR="$2"
                shift 2
                ;;
            --bundle-dir=*)
                BUNDLE_DIR="${1#*=}"
                shift
                ;;
            --base-packages)
                [ "$#" -ge 2 ] || die "Missing value for --base-packages"
                BASE_PACKAGES="$2"
                shift 2
                ;;
            --base-packages=*)
                BASE_PACKAGES="${1#*=}"
                shift
                ;;
            --expected-arch)
                [ "$#" -ge 2 ] || die "Missing value for --expected-arch"
                EXPECTED_ARCH="$2"
                shift 2
                ;;
            --expected-arch=*)
                EXPECTED_ARCH="${1#*=}"
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                die "Unknown argument: $1"
                ;;
        esac
    done

    [ -n "$BUNDLE_DIR" ] || die "--bundle-dir is required"
    [ -n "$BASE_PACKAGES" ] || die "--base-packages is required"
    [ -d "$BUNDLE_DIR" ] || die "Bundle directory does not exist: $BUNDLE_DIR"
    [ -f "$BUNDLE_DIR/rune" ] || die "Executable not found in bundle: $BUNDLE_DIR/rune"
    [ -d "$BUNDLE_DIR/lib" ] || die "Library directory not found in bundle: $BUNDLE_DIR/lib"
    [ -s "$BASE_PACKAGES" ] || die "Base packages file is missing or empty: $BASE_PACKAGES"
}

resolve_expected_elf_machine() {
    local raw_arch="${EXPECTED_ARCH:-$(uname -m)}"
    case "$raw_arch" in
        x64|amd64|x86_64)
            EXPECTED_ELF_MACHINE="Advanced Micro Devices X86-64"
            ;;
        arm64|aarch64)
            EXPECTED_ELF_MACHINE="AArch64"
            ;;
        *)
            die "Unsupported architecture: $raw_arch"
            ;;
    esac

    log "Expected ELF machine: $EXPECTED_ELF_MACHINE (arch=$raw_arch)"
    if command -v dpkg-architecture >/dev/null 2>&1; then
        log "Host multiarch: $(dpkg-architecture -qDEB_HOST_MULTIARCH)"
    fi
}

# Core libc, C++ runtime, and graphics/windowing ABI libraries provided by
# Steam Linux Runtime 4 (or host driver passthrough) that must never be bundled.
is_forbidden_lib() {
    local soname="$1"
    case "$soname" in
        linux-vdso.so.*|ld-linux*.so.*|\
        libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|\
        libresolv.so.*|libnsl.so.*|libnss_*.so.*|\
        libstdc++.so.*|libgcc_s.so.*|\
        libGL.so.*|libGLX.so.*|libGLdispatch.so.*|\
        libEGL.so.*|libGLESv*.so.*|libvulkan.so.*|\
        libdrm.so.*|libdrm_*.so.*|libgbm.so.*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

collect_extra_package_files() {
    dpkg-query -W -f='${binary:Package}\n' | sort -u > "$WORK_DIR/current-packages.txt"
    comm -13 <(sort -u "$BASE_PACKAGES") "$WORK_DIR/current-packages.txt" > "$WORK_DIR/extra-packages.txt"

    if [ ! -s "$WORK_DIR/extra-packages.txt" ]; then
        die "No newly installed packages found relative to $BASE_PACKAGES"
    fi
    log "Detected $(wc -l < "$WORK_DIR/extra-packages.txt") newly installed package(s)"

    : > "$WORK_DIR/extra-lib-paths.raw"
    : > "$WORK_DIR/extra-schema-files.raw"

    local pkg pkg_path
    while IFS= read -r pkg; do
        [ -n "$pkg" ] || continue
        while IFS= read -r pkg_path; do
            [ -n "$pkg_path" ] || continue
            case "$pkg_path" in
                *.so|*.so.*)
                    if [ -f "$pkg_path" ]; then
                        readlink -f "$pkg_path" >> "$WORK_DIR/extra-lib-paths.raw"
                    fi
                    ;;
                */usr/share/glib-2.0/schemas/*.gschema.xml|*/usr/share/glib-2.0/schemas/*.gschema.override)
                    if [ -f "$pkg_path" ]; then
                        printf '%s\n' "$pkg_path" >> "$WORK_DIR/extra-schema-files.raw"
                    fi
                    ;;
            esac
        done < <(dpkg -L "$pkg" 2>/dev/null || true)
    done < "$WORK_DIR/extra-packages.txt"

    sort -u "$WORK_DIR/extra-lib-paths.raw" > "$WORK_DIR/extra-lib-paths.txt"
    sort -u "$WORK_DIR/extra-schema-files.raw" > "$WORK_DIR/extra-schema-files.txt"
}

collect_bundle_targets() {
    BUNDLE_TARGETS=("$BUNDLE_DIR/rune")
    local lib_file
    while IFS= read -r -d '' lib_file; do
        BUNDLE_TARGETS+=("$lib_file")
    done < <(find "$BUNDLE_DIR/lib" -maxdepth 1 \( -type f -o -type l \) \( -name '*.so' -o -name '*.so.*' \) -print0 | LC_ALL=C sort -z)
}

bundle_transitive_libs() {
    local max_passes=20
    local pass added target ldd_out line soname resolved real_path

    for ((pass = 1; pass <= max_passes; pass++)); do
        added=0
        collect_bundle_targets

        for target in "${BUNDLE_TARGETS[@]}"; do
            ldd_out="$(LD_LIBRARY_PATH="$BUNDLE_DIR_REAL/lib" ldd "$target")" || die "ldd failed for $target"
            while IFS= read -r line; do
                if [[ "$line" == *"=> not found"* ]]; then
                    die "Missing shared library in $target: $line"
                fi
                if [[ "$line" =~ ^[[:space:]]*([^[:space:]]+)[[:space:]]*=\>[[:space:]]*(/.*)[[:space:]]+\(0x[0-9a-fA-F]+\)[[:space:]]*$ ]]; then
                    soname="${BASH_REMATCH[1]}"
                    resolved="${BASH_REMATCH[2]}"
                    resolved="${resolved%"${resolved##*[![:space:]]}"}"
                    real_path="$(readlink -f "$resolved")"

                    if [[ "$real_path" == "$BUNDLE_DIR_REAL/"* ]]; then
                        continue
                    fi
                    if is_forbidden_lib "$soname"; then
                        continue
                    fi
                    if grep -Fxq "$real_path" "$WORK_DIR/extra-lib-paths.txt" && [ ! -e "$BUNDLE_DIR/lib/$soname" ]; then
                        cp -L "$real_path" "$BUNDLE_DIR/lib/$soname"
                        chmod 0644 "$BUNDLE_DIR/lib/$soname"
                        log "Bundled extra runtime library: $soname ($real_path)"
                        added=$((added + 1))
                    fi
                fi
            done <<< "$ldd_out"
        done

        log "Transitive ldd pass $pass: bundled $added library(ies)"
        if [ "$added" -eq 0 ]; then
            return 0
        fi
    done

    die "Transitive library bundling did not converge within $max_passes passes"
}

bundle_gsettings_schemas() {
    local schema_dir="$BUNDLE_DIR/share/glib-2.0/schemas"
    local schema_file copied=0

    if [ ! -s "$WORK_DIR/extra-schema-files.txt" ]; then
        die "No GSettings schema files found in newly installed packages"
    fi

    mkdir -p "$schema_dir"
    while IFS= read -r schema_file; do
        [ -n "$schema_file" ] || continue
        cp -L "$schema_file" "$schema_dir/"
        copied=$((copied + 1))
    done < "$WORK_DIR/extra-schema-files.txt"

    log "Copied $copied GSettings schema file(s); compiling..."
    glib-compile-schemas "$schema_dir"

    [ -s "$schema_dir/gschemas.compiled" ] || die "Compiled GSettings schema file is missing or empty: $schema_dir/gschemas.compiled"
    if ! GSETTINGS_SCHEMA_DIR="$schema_dir" gsettings list-schemas | grep -Fxq 'org.gtk.Settings.FileChooser'; then
        die "Expected schema org.gtk.Settings.FileChooser not found in $schema_dir"
    fi
}

copy_launch_scripts() {
    local steam_scripts_dir="$ROOT_DIR/scripts/steam"
    [ -d "$steam_scripts_dir" ] || die "Steam scripts directory not found: $steam_scripts_dir"

    cp -a "$steam_scripts_dir/." "$BUNDLE_DIR/"
    chmod +x \
        "$BUNDLE_DIR/steam_launch.sh" \
        "$BUNDLE_DIR/xdg-user-dir"
}

verify_bundle() {
    local required_libs=(
        "libhub.so"
        "libflutter_linux_gtk.so"
        "libapp.so"
        "libgtk-3.so.0"
        "libgdk-3.so.0"
        "libepoxy.so.0"
        "libayatana-appindicator3.so.1"
    )
    local lib
    for lib in "${required_libs[@]}"; do
        [ -f "$BUNDLE_DIR/lib/$lib" ] || die "Required bundled library is missing: $BUNDLE_DIR/lib/$lib"
    done

    local entry base
    while IFS= read -r -d '' entry; do
        base="$(basename "$entry")"
        if is_forbidden_lib "$base"; then
            die "Forbidden library found in bundle/lib: $base"
        fi
    done < <(find "$BUNDLE_DIR/lib" -maxdepth 1 \( -type f -o -type l \) -print0)

    local elf_targets=(
        "$BUNDLE_DIR/rune"
        "$BUNDLE_DIR/lib/libhub.so"
        "$BUNDLE_DIR/lib/libflutter_linux_gtk.so"
        "$BUNDLE_DIR/lib/libgtk-3.so.0"
    )
    local bin machine newest_glibc
    for bin in "${elf_targets[@]}"; do
        machine="$(LC_ALL=C readelf -h "$bin" | awk -F: '/Machine:/ { gsub(/^[[:space:]]+|[[:space:]]+$/, "", $2); print $2 }')"
        if [ "$machine" != "$EXPECTED_ELF_MACHINE" ]; then
            die "Unexpected ELF machine for $bin: got '$machine', expected '$EXPECTED_ELF_MACHINE'"
        fi
        newest_glibc="$(readelf --dyn-syms -W "$bin" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -n 1 || true)"
        log "$bin: machine='$machine', newest_glibc='${newest_glibc:-none}'"
    done

    collect_bundle_targets
    local target ldd_out line resolved real_path
    for target in "${BUNDLE_TARGETS[@]}"; do
        ldd_out="$(LD_LIBRARY_PATH="$BUNDLE_DIR_REAL/lib" ldd "$target")" || die "Final ldd check failed for $target"
        while IFS= read -r line; do
            if [[ "$line" == *"=> not found"* ]]; then
                die "Unresolved shared library after bundling in $target: $line"
            fi
            if [[ "$line" =~ ^[[:space:]]*([^[:space:]]+)[[:space:]]*=\>[[:space:]]*(/.*)[[:space:]]+\(0x[0-9a-fA-F]+\)[[:space:]]*$ ]]; then
                resolved="${BASH_REMATCH[2]}"
                resolved="${resolved%"${resolved##*[![:space:]]}"}"
                real_path="$(readlink -f "$resolved")"
                if [[ "$real_path" != "$BUNDLE_DIR_REAL/"* ]] && grep -Fxq "$real_path" "$WORK_DIR/extra-lib-paths.txt"; then
                    die "Extra-package library resolved outside bundle in $target: $line"
                fi
            fi
        done <<< "$ldd_out"
    done

    local script_file
    for script_file in \
        "$BUNDLE_DIR/steam_launch.sh" \
        "$BUNDLE_DIR/xdg-user-dir"; do
        [ -x "$script_file" ] || die "Launch script is not executable: $script_file"
        sh -n "$script_file" || die "POSIX shell syntax check failed for $script_file"
    done

    log "Verification passed."
}

main() {
    parse_args "$@"
    ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
    BUNDLE_DIR_REAL="$(readlink -f "$BUNDLE_DIR")"
    WORK_DIR="$(mktemp -d)"
    trap 'rm -rf "$WORK_DIR"' EXIT

    resolve_expected_elf_machine
    collect_extra_package_files
    bundle_transitive_libs
    bundle_gsettings_schemas
    copy_launch_scripts
    verify_bundle
}

main "$@"
