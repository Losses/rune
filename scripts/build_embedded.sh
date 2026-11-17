#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARCH="arm64"
OUTPUT_DIR="$ROOT_DIR/build/embedded"

# The flutter-pi commit the bundle's embedder is built from.
FLUTTER_PI_REV="edda7f9a216a2fd60586ef350aac84ebaa847bb0"

for arg in "$@"; do
    case "$arg" in
        --arch=*)
            ARCH="${arg#*=}"
            ;;
        --output-dir=*)
            OUTPUT_DIR="${arg#*=}"
            ;;
        --help)
            echo "Usage: ./scripts/build_embedded.sh [--arch=arm64|armhf] [--output-dir=path]"
            exit 0
            ;;
    esac
done

echo "==============================================="
echo " Building Rune Embedded Bundle ($ARCH) "
echo "==============================================="

case "$ARCH" in
    arm64|aarch64)
        TARGET_ARCH="arm64"
        RUST_TARGET="aarch64-unknown-linux-gnu"
        CC_TARGET="aarch64-linux-gnu-gcc"
        CXX_TARGET="aarch64-linux-gnu-g++"
        PKG_ARCH="arm64"
        MULTIARCH="aarch64-linux-gnu"
        # The engine builds of ardera/flutter-ci for this CPU.
        ENGINE_FLAVOR="aarch64-generic"
        ;;
    arm|armhf|armv7)
        TARGET_ARCH="arm"
        RUST_TARGET="armv7-unknown-linux-gnueabihf"
        CC_TARGET="arm-linux-gnueabihf-gcc"
        CXX_TARGET="arm-linux-gnueabihf-g++"
        PKG_ARCH="armhf"
        MULTIARCH="arm-linux-gnueabihf"
        ENGINE_FLAVOR="armv7-generic"
        ;;
    *)
        echo "Error: Unsupported architecture: $ARCH" >&2
        exit 1
        ;;
esac

HOST_ARCH="$(uname -m)"
IS_CROSS=false
if [ "$HOST_ARCH" != "aarch64" ] && [ "$TARGET_ARCH" = "arm64" ]; then
    IS_CROSS=true
elif [ "$HOST_ARCH" != "armv7l" ] && [ "$TARGET_ARCH" = "arm" ]; then
    IS_CROSS=true
fi

# gen_snapshot runs on the build machine: ardera/flutter-ci names its builds
# by the host they run on.
case "$HOST_ARCH" in
    x86_64) GEN_SNAPSHOT_HOST="X64" ;;
    aarch64) GEN_SNAPSHOT_HOST="ARM64" ;;
    *)
        echo "Error: No gen_snapshot builds for a $HOST_ARCH build machine" >&2
        exit 1
        ;;
esac

BUNDLE_DIR="$OUTPUT_DIR/rune-embedded-$PKG_ARCH"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/bin" "$BUNDLE_DIR/lib" "$BUNDLE_DIR/data"

WORK_DIR="$(mktemp -d /tmp/rune-embedded-build.XXXXXX)"
trap 'rm -rf "$WORK_DIR"' EXIT

echo ">> 1. Generating Rinf bindings..."
cargo install rinf_cli --version 8.11.0 --locked 2>/dev/null || true
flutter pub get
rinf gen

echo ">> 2. Compiling Rust backend (libhub.so) for $RUST_TARGET..."
if [ "$IS_CROSS" = true ]; then
    if [ "$TARGET_ARCH" = "arm" ]; then
        export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER="$CC_TARGET"
        export CC_armv7_unknown_linux_gnueabihf="$CC_TARGET"
        export CXX_armv7_unknown_linux_gnueabihf="$CXX_TARGET"
        export RUSTFLAGS="-C target-feature=+neon ${RUSTFLAGS:-}"
        export CFLAGS_armv7_unknown_linux_gnueabihf="-mfpu=neon"
        export CXXFLAGS_armv7_unknown_linux_gnueabihf="-mfpu=neon"
    else
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$CC_TARGET"
        export CC_aarch64_unknown_linux_gnu="$CC_TARGET"
        export CXX_aarch64_unknown_linux_gnu="$CXX_TARGET"
    fi
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_PATH="/usr/lib/$MULTIARCH/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
    cargo build --release --target "$RUST_TARGET" -p hub
    cp "$ROOT_DIR/target/$RUST_TARGET/release/libhub.so" "$BUNDLE_DIR/lib/libhub.so"
else
    cargo build --release -p hub
    cp "$ROOT_DIR/target/release/libhub.so" "$BUNDLE_DIR/lib/libhub.so"
fi

echo ">> 3. Fetching the Flutter engine for $ENGINE_FLAVOR..."
# The engine, gen_snapshot and the app snapshot must come from the same engine
# revision as the Flutter SDK that compiles the app.
ENGINE_REVISION="$(flutter --version --machine | jq -r .engineRevision)"
ENGINE_URL="https://github.com/ardera/flutter-ci/releases/download/engine/$ENGINE_REVISION"
for asset in \
    "engine-$ENGINE_FLAVOR-release.tar.xz" \
    "gen-snapshot-Linux-$GEN_SNAPSHOT_HOST-$ENGINE_FLAVOR-release.tar.xz" \
    "universal.tar.xz"; do
    if ! curl -fsSL "$ENGINE_URL/$asset" -o "$WORK_DIR/$asset"; then
        echo "Error: ardera/flutter-ci has no $asset for engine $ENGINE_REVISION" >&2
        exit 1
    fi
done
mkdir -p "$WORK_DIR/engine" "$WORK_DIR/gen_snapshot" "$WORK_DIR/universal"
tar -xf "$WORK_DIR/engine-$ENGINE_FLAVOR-release.tar.xz" -C "$WORK_DIR/engine"
tar -xf "$WORK_DIR/gen-snapshot-Linux-$GEN_SNAPSHOT_HOST-$ENGINE_FLAVOR-release.tar.xz" -C "$WORK_DIR/gen_snapshot"
tar -xf "$WORK_DIR/universal.tar.xz" -C "$WORK_DIR/universal"
# flutter-pi loads the engine inside the bundle before any other on the
# system, so an older engine installed system-wide is not picked up.
cp "$WORK_DIR/engine/libflutter_engine.so" "$BUNDLE_DIR/data/libflutter_engine.so"
cp "$WORK_DIR/engine/libflutter_engine.so" "$BUNDLE_DIR/lib/libflutter_engine.so"
cp "$WORK_DIR/universal/icudtl.dat" "$BUNDLE_DIR/data/icudtl.dat"

echo ">> 4. Building the Flutter assets and the AOT snapshot..."
# Assets and the kernel are the same for every CPU. Flutter has no 32-bit ARM
# Linux target, so the kernel is built for linux-arm64 either way; assemble
# also generates the registrant of the Dart plugins (path_provider and others).
flutter build bundle --release --target-platform linux-arm64 --asset-dir "$BUNDLE_DIR/data"
rm -f "$BUNDLE_DIR/data/kernel_blob.bin"
flutter assemble \
    --output="$WORK_DIR/assemble" \
    -dTargetPlatform=linux-arm64 \
    -dBuildMode=release \
    -dTargetFile=lib/main.dart \
    -dTrackWidgetCreation=false \
    kernel_snapshot_program
APP_DILL="$(ls -t .dart_tool/flutter_build/*/app.dill | head -n 1)"
chmod +x "$WORK_DIR/gen_snapshot/gen_snapshot"
"$WORK_DIR/gen_snapshot/gen_snapshot" \
    --deterministic \
    --snapshot_kind=app-aot-elf \
    --elf="$BUNDLE_DIR/data/app.so" \
    --strip \
    "$APP_DILL"
if [ ! -f "$BUNDLE_DIR/data/app.so" ]; then
    echo "Error: app.so was not generated!" >&2
    exit 1
fi

echo ">> 5. Building flutter-pi DRM/KMS embedder..."
git clone https://github.com/ardera/flutter-pi.git "$WORK_DIR/flutter-pi"
git -C "$WORK_DIR/flutter-pi" checkout --quiet "$FLUTTER_PI_REV"
# The stubs answer the desktop plugins' channels and give bitsdojo_window the
# window API it looks up; the patch compiles them in. flutter-pi already links
# with -rdynamic, which exports that API to the app.
cp "$ROOT_DIR/linux/embedded/desktop_stubs.c" "$WORK_DIR/flutter-pi/src/plugins/desktop_stubs.c"
git -C "$WORK_DIR/flutter-pi" apply "$ROOT_DIR/linux/embedded/flutter-pi.patch"
# --partial-repaint: Flutter draws only what changed since the buffer it draws
# into was last shown, instead of the whole screen every frame.
git -C "$WORK_DIR/flutter-pi" apply "$ROOT_DIR/linux/embedded/flutter-pi-partial-repaint.patch"
# The display goes off after the time the app sets on the rune/display_power
# channel, and Flutter is told the app is paused meanwhile.
git -C "$WORK_DIR/flutter-pi" apply "$ROOT_DIR/linux/embedded/flutter-pi-display-blank.patch"

CMAKE_FLAGS=(-DCMAKE_BUILD_TYPE=Release -DENABLE_TESTS=OFF)
if [ "$IS_CROSS" = true ]; then
    if [ "$TARGET_ARCH" = "arm" ]; then
        PROCESSOR="armv7-a"
    else
        PROCESSOR="aarch64"
    fi
    export PKG_CONFIG_ALLOW_CROSS=1
    export PKG_CONFIG_PATH="/usr/lib/$MULTIARCH/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
    export PKG_CONFIG_LIBDIR="/usr/lib/$MULTIARCH/pkgconfig:/usr/share/pkgconfig"
    CMAKE_FLAGS+=(
        -DCMAKE_C_COMPILER="$CC_TARGET"
        -DCMAKE_CXX_COMPILER="$CXX_TARGET"
        -DCMAKE_SYSTEM_NAME=Linux
        -DCMAKE_SYSTEM_PROCESSOR="$PROCESSOR"
    )
fi

cmake -B "$WORK_DIR/flutter-pi/build" -S "$WORK_DIR/flutter-pi" "${CMAKE_FLAGS[@]}"
cmake --build "$WORK_DIR/flutter-pi/build" --target flutter-pi -j"$(nproc)"
cp "$WORK_DIR/flutter-pi/build/flutter-pi" "$BUNDLE_DIR/bin/flutter-pi"
chmod +x "$BUNDLE_DIR/bin/flutter-pi"

echo ">> 6. Copying embedded deployment assets..."
cp "$ROOT_DIR/linux/embedded/rune-kiosk.service" "$BUNDLE_DIR/"
cp "$ROOT_DIR/linux/embedded/rune-kiosk.conf" "$BUNDLE_DIR/"
cp "$ROOT_DIR/linux/embedded/install.sh" "$BUNDLE_DIR/"
cp "$ROOT_DIR/linux/embedded/rune-gpu-devfreq" "$BUNDLE_DIR/bin/"
cp "$ROOT_DIR/linux/embedded/rune-cpu-cpufreq" "$BUNDLE_DIR/bin/"
cp "$ROOT_DIR/linux/embedded/rune-unused-devices" "$BUNDLE_DIR/bin/"
cp "$ROOT_DIR/linux/embedded/rune-soc-profile" "$BUNDLE_DIR/bin/"
cp -r "$ROOT_DIR/linux/embedded/soc" "$BUNDLE_DIR/"
cp "$ROOT_DIR/linux/embedded/rune-kiosk-desktop" "$BUNDLE_DIR/bin/"
chmod +x "$BUNDLE_DIR/install.sh" "$BUNDLE_DIR/bin/rune-gpu-devfreq" "$BUNDLE_DIR/bin/rune-cpu-cpufreq" \
    "$BUNDLE_DIR/bin/rune-unused-devices" "$BUNDLE_DIR/bin/rune-kiosk-desktop"

echo ">> 7. Packaging bundle..."
VERSION="$(git describe --tags --always || echo "latest")"
TARBALL_NAME="Rune-$VERSION-embedded-$PKG_ARCH.tar.gz"

tar -czf "$OUTPUT_DIR/$TARBALL_NAME" -C "$OUTPUT_DIR" "rune-embedded-$PKG_ARCH"

echo "==============================================="
echo " Build Complete:"
echo " - $OUTPUT_DIR/$TARBALL_NAME"
echo "==============================================="
