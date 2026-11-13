#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

ARCH="arm64"
OUTPUT_DIR="$ROOT_DIR/build/embedded"
RELEASE=true

for arg in "$@"; do
    case "$arg" in
        --arch=*)
            ARCH="${arg#*=}"
            ;;
        --output-dir=*)
            OUTPUT_DIR="${arg#*=}"
            ;;
        --debug)
            RELEASE=false
            ;;
        --help)
            echo "Usage: ./scripts/build_embedded.sh [--arch=arm64|armhf] [--output-dir=path] [--debug]"
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
        ;;
    arm|armhf|armv7)
        TARGET_ARCH="arm"
        RUST_TARGET="armv7-unknown-linux-gnueabihf"
        CC_TARGET="arm-linux-gnueabihf-gcc"
        CXX_TARGET="arm-linux-gnueabihf-g++"
        PKG_ARCH="armhf"
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

BUNDLE_DIR="$OUTPUT_DIR/rune-embedded-$PKG_ARCH"
rm -rf "$BUNDLE_DIR"
mkdir -p "$BUNDLE_DIR/bin" "$BUNDLE_DIR/lib" "$BUNDLE_DIR/data"

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
        export PKG_CONFIG_ALLOW_CROSS=1
        if [ -d "/usr/lib/arm-linux-gnueabihf/pkgconfig" ]; then
            export PKG_CONFIG_PATH="/usr/lib/arm-linux-gnueabihf/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
        fi
        export RUSTFLAGS="-C target-feature=+neon ${RUSTFLAGS:-}"
        export CFLAGS_armv7_unknown_linux_gnueabihf="-mfpu=neon"
        export CXXFLAGS_armv7_unknown_linux_gnueabihf="-mfpu=neon"
    else
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="$CC_TARGET"
        export CC_aarch64_unknown_linux_gnu="$CC_TARGET"
        export CXX_aarch64_unknown_linux_gnu="$CXX_TARGET"
        export PKG_CONFIG_ALLOW_CROSS=1
        if [ -d "/usr/lib/aarch64-linux-gnu/pkgconfig" ]; then
            export PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:${PKG_CONFIG_PATH:-}"
        fi
    fi
    cargo build --release --target "$RUST_TARGET" -p hub
    cp "$ROOT_DIR/target/$RUST_TARGET/release/libhub.so" "$BUNDLE_DIR/lib/libhub.so"
else
    cargo build --release -p hub
    cp "$ROOT_DIR/target/release/libhub.so" "$BUNDLE_DIR/lib/libhub.so"
fi

if [ "$TARGET_ARCH" = "arm64" ]; then
    echo ">> 3. Building Flutter Dart AOT Bundle via flutter build linux..."
    flutter build linux --release --verbose

    BUNDLE_SRC="build/linux/arm64/release/bundle"
    if [ ! -d "$BUNDLE_SRC" ]; then
        BUNDLE_SRC="$(find build/linux -type d -name "bundle" | head -n 1)"
    fi

    echo "Copying assets from $BUNDLE_SRC..."
    cp -r "$BUNDLE_SRC/data/flutter_assets"/* "$BUNDLE_DIR/data/"
    test -f "$BUNDLE_SRC/data/icudtl.dat" && cp "$BUNDLE_SRC/data/icudtl.dat" "$BUNDLE_DIR/data/" || true
    test -f "$BUNDLE_SRC/lib/libapp.so" && cp "$BUNDLE_SRC/lib/libapp.so" "$BUNDLE_DIR/data/app.so" || true

    echo ">> 3.1 Fetching libflutter_engine.so for arm64..."
    FLUTTER_VERSION="$(flutter --version 2>/dev/null | head -n 1 | awk '{print $2}')"
    ENGINE_URL="https://github.com/ardera/flutter-engine-binaries-for-arm/releases/download/v${FLUTTER_VERSION}/engine-binaries-linux-arm64.tar.xz"
    curl -sSL "$ENGINE_URL" -o /tmp/engine.tar.xz 2>/dev/null || true
    if [ -f /tmp/engine.tar.xz ] && tar -tf /tmp/engine.tar.xz >/dev/null 2>&1; then
        tar -xf /tmp/engine.tar.xz -C "$BUNDLE_DIR/lib/" libflutter_engine.so.release 2>/dev/null || true
        test -f "$BUNDLE_DIR/lib/libflutter_engine.so.release" && mv "$BUNDLE_DIR/lib/libflutter_engine.so.release" "$BUNDLE_DIR/lib/libflutter_engine.so"
        rm -f /tmp/engine.tar.xz
    fi
else
    echo ">> 3. Building Flutter Dart AOT Bundle for armhf via flutterpi_tool..."
    if ! command -v flutterpi_tool >/dev/null 2>&1; then
        echo "Installing flutterpi_tool..."
        flutter pub global activate flutterpi_tool
        export PATH="$PATH:$HOME/.pub-cache/bin"
    fi

    # Ensure dependencies are resolved
    flutter pub get

    flutterpi_tool build --arch=arm --cpu=generic --release

    echo "Copying assets from build/flutter_assets..."
    cp -r build/flutter_assets/* "$BUNDLE_DIR/data/"
    if [ -f "$BUNDLE_DIR/data/libflutter_engine.so" ]; then
        cp "$BUNDLE_DIR/data/libflutter_engine.so" "$BUNDLE_DIR/lib/libflutter_engine.so"
    fi
    rm -f "$BUNDLE_DIR/data/flutter-pi" || true

    if [ ! -f "$BUNDLE_DIR/data/app.so" ]; then
        echo "Error: app.so was not generated by flutterpi_tool!" >&2
        exit 1
    fi
fi

echo ">> 4. Building flutter-pi DRM/KMS embedder..."
BUILD_TMP="$(mktemp -d /tmp/flutter-pi-build.XXXXXX)"
trap 'rm -rf "$BUILD_TMP"' EXIT

git clone --depth 1 https://github.com/ardera/flutter-pi.git "$BUILD_TMP/src"
cp "$ROOT_DIR/linux/embedded/desktop_stubs.c" "$BUILD_TMP/src/src/plugins/desktop_stubs.c"

# Add -rdynamic to export symbols for bitsdojo_window
sed -i 's/target_link_libraries(flutter-pi/target_link_options(flutter-pi PRIVATE -rdynamic)\ntarget_link_libraries(flutter-pi/' "$BUILD_TMP/src/CMakeLists.txt"

CMAKE_FLAGS=(-DCMAKE_BUILD_TYPE=Release -DENABLE_TESTS=OFF)
if [ "$IS_CROSS" = true ]; then
    if [ "$TARGET_ARCH" = "arm" ]; then
        PROCESSOR="armv7-a"
        export PKG_CONFIG_PATH="/usr/lib/arm-linux-gnueabihf/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
        export PKG_CONFIG_LIBDIR="/usr/lib/arm-linux-gnueabihf/pkgconfig"
    else
        PROCESSOR="aarch64"
        export PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig:${PKG_CONFIG_PATH:-}"
        export PKG_CONFIG_LIBDIR="/usr/lib/aarch64-linux-gnu/pkgconfig"
    fi
    export PKG_CONFIG_ALLOW_CROSS=1
    CMAKE_FLAGS+=(
        -DCMAKE_C_COMPILER="$CC_TARGET"
        -DCMAKE_CXX_COMPILER="$CXX_TARGET"
        -DCMAKE_SYSTEM_NAME=Linux
        -DCMAKE_SYSTEM_PROCESSOR="$PROCESSOR"
    )
fi

cmake -B "$BUILD_TMP/build" -S "$BUILD_TMP/src" "${CMAKE_FLAGS[@]}"
cmake --build "$BUILD_TMP/build" --target flutter-pi -j"$(nproc)"
cp "$BUILD_TMP/build/flutter-pi" "$BUNDLE_DIR/bin/flutter-pi"
chmod +x "$BUNDLE_DIR/bin/flutter-pi"

echo ">> 5. Copying embedded deployment assets..."
cp "$ROOT_DIR/linux/embedded/rune-kiosk.service" "$BUNDLE_DIR/"
cp "$ROOT_DIR/linux/embedded/install.sh" "$BUNDLE_DIR/"
chmod +x "$BUNDLE_DIR/install.sh"

echo ">> 6. Packaging bundle..."
VERSION="$(git describe --tags --always || echo "latest")"
TARBALL_NAME="Rune-$VERSION-embedded-$PKG_ARCH.tar.gz"
ZIP_NAME="Rune-$VERSION-embedded-$PKG_ARCH.zip"

tar -czf "$OUTPUT_DIR/$TARBALL_NAME" -C "$OUTPUT_DIR" "rune-embedded-$PKG_ARCH"
if command -v zip >/dev/null 2>&1; then
    (cd "$OUTPUT_DIR" && zip -r -q "$ZIP_NAME" "rune-embedded-$PKG_ARCH")
fi

echo "==============================================="
echo " Build Complete:"
echo " - $OUTPUT_DIR/$TARBALL_NAME"
echo " - $OUTPUT_DIR/$ZIP_NAME"
echo "==============================================="
