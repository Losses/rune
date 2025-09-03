#!/bin/bash
WORK_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

# Check if we have ANDROID_NDK_PATH set
if [ -z "$ANDROID_NDK_PATH" ]; then
    echo "ERROR: ANDROID_NDK_PATH env variable not set"
    exit 1
fi

# Use NDK toolchain corresponding to OS
OS_TYPE=$(uname -s)
if [ "$OS_TYPE" = "Darwin" ]; then
    PLATFORM="darwin-x86_64"
else
    PLATFORM="linux-x86_64"
fi

# Setup OpenSSL
source "$WORK_DIR/setup-openssl.sh"
download_openssl
if [ $? -eq 0 ]; then
    echo "OpenSSL downloaded successfully (or exists already)"
else
    echo "OpenSSL download failed, please check output above for details"
    exit 1
fi

# Set env variables
export PKG_CONFIG_PATH="$ANDROID_NDK_PATH/toolchains/llvm/prebuilt/$PLATFORM/bin"
export PKG_CONFIG_SYSROOT_DIR="$ANDROID_NDK_PATH/toolchains/llvm/prebuilt/$PLATFORM/sysroot"
export ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR="$WORK_DIR/openssl/armeabi-v7a"
export AARCH64_LINUX_ANDROID_OPENSSL_DIR="$WORK_DIR/openssl/arm64-v8a"
export I686_LINUX_ANDROID_OPENSSL_DIR="$WORK_DIR/openssl/x86"
export X86_64_LINUX_ANDROID_OPENSSL_DIR="$WORK_DIR/openssl/x86_64"

if [[ "${NIX_NIX_DEV_SHELL}" = "true" ]]; then
    export ANDROID_NDK_ROOT=$NIX_ANDROID_NDK_ROOT
    export CFLAGS=$NIX_CFLAGS
    export CXXFLAGS=$NIX_CXXFLAGS
    export BINDGEN_EXTRA_CLANG_ARGS=$NIX_BINDGEN_EXTRA_CLANG_ARGS
    export RUSTFLAGS=$NIX_RUSTFLAGS
    export CMAKE_TOOLCHAIN_FILE=$NIX_CMAKE_TOOLCHAIN_FILE
    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=$NIX_ANDROID_SDK/share/android-sdk/build-tools/34.0.0/aapt2"
    export PATH=$NIX_TOOLCHAIN_BIN_PATH:$NIX_ANDROID_SDK/share/android-sdk/platform-tools:$NIX_ANDROID_SDK/share/android-sdk/tools:$NIX_ANDROID_SDK/share/android-sdk/tools/bin:$PATH
    export _JAVA_OPTIONS="-Dorg.gradle.projectcachedir=$(mktemp -d)"

    echo "=================="
    echo "=== Nix Shell Environment Setup ==="

    echo "ANDROID_NDK_PATH: $ANDROID_NDK_PATH"
    echo "ANDROID_NDK_ROOT: $ANDROID_NDK_ROOT"
    echo "CFLAGS: $CFLAGS"
    echo "CXXFLAGS: $CXXFLAGS"
    echo "BINDGEN_EXTRA_CLANG_ARGS: $BINDGEN_EXTRA_CLANG_ARGS"
    echo "RUSTFLAGS: $RUSTFLAGS"
    echo "CMAKE_TOOLCHAIN_FILE: $CMAKE_TOOLCHAIN_FILE"
    
    echo "=== Nix Shell JDK Setup ==="

    flutter config --jdk-dir $NIX_PINNED_JDK
    echo "=================="
fi

# Print env variables for debugging purpose
echo "=== General Environment Setup ==="
echo "WORK_DIR: $WORK_DIR"
echo "PKG_CONFIG_PATH: $PKG_CONFIG_PATH"
echo "PKG_CONFIG_SYSROOT_DIR: $PKG_CONFIG_SYSROOT_DIR"
echo "ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR: $ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR"
echo "AARCH64_LINUX_ANDROID_OPENSSL_DIR: $AARCH64_LINUX_ANDROID_OPENSSL_DIR"
echo "I686_LINUX_ANDROID_OPENSSL_DIR: $I686_LINUX_ANDROID_OPENSSL_DIR"
echo "X86_64_LINUX_ANDROID_OPENSSL_DIR: $X86_64_LINUX_ANDROID_OPENSSL_DIR"
echo "=================="

# Check if Flutter is present
if ! command -v flutter &> /dev/null; then
    echo "ERROR: Flutter command not found"
    exit 1
fi

# Exec Flutter build
echo "Starting Flutter build..."
flutter build apk --release --split-per-abi

# Report build result
if [ $? -eq 0 ]; then
    echo "Built successfully!"
else
    echo "ERROR: build failed, please check output above for details"
    exit 1
fi
