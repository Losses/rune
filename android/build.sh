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

# Common writable directories setup for both Nix and non-Nix environments
GRADLE_CACHE_DIR="$WORK_DIR/.gradle_cache"
PUB_CACHE_DIR="$WORK_DIR/.pub_cache"
FLUTTER_CACHE_DIR="$WORK_DIR/.flutter_cache"

# Create necessary writable directories
mkdir -p "$GRADLE_CACHE_DIR"
mkdir -p "$PUB_CACHE_DIR" 
mkdir -p "$FLUTTER_CACHE_DIR"
mkdir -p "$WORK_DIR/tmp"

# Set common cache directories
export PUB_CACHE="$PUB_CACHE_DIR"
export TMPDIR="$WORK_DIR/tmp"
export TEMP="$WORK_DIR/tmp"
export TMP="$WORK_DIR/tmp"

if [[ "${NIX_NIX_DEV_SHELL}" = "true" ]]; then
    export ANDROID_NDK_ROOT=$NIX_ANDROID_NDK_ROOT
    export CFLAGS=$NIX_CFLAGS
    export CXXFLAGS=$NIX_CXXFLAGS
    export BINDGEN_EXTRA_CLANG_ARGS=$NIX_BINDGEN_EXTRA_CLANG_ARGS
    export RUSTFLAGS=$NIX_RUSTFLAGS
    export CMAKE_TOOLCHAIN_FILE=$NIX_CMAKE_TOOLCHAIN_FILE
    
    # Base GRADLE_OPTS for Nix environment
    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=$NIX_ANDROID_SDK/share/android-sdk/build-tools/34.0.0/aapt2"

    # Set Gradle home to writable location for all Nix environments
    export GRADLE_USER_HOME="$GRADLE_CACHE_DIR"
    
    # Additional Gradle configuration to prevent writing to read-only Nix store
    export GRADLE_OPTS="$GRADLE_OPTS -Dgradle.user.home=$GRADLE_CACHE_DIR -Duser.home=$HOME -Djava.io.tmpdir=$WORK_DIR/tmp"

    # Create global gradle.properties for consistent behavior
    mkdir -p "$HOME/.gradle"
    cat > "$HOME/.gradle/gradle.properties" << EOF
systemProp.gradle.user.home=$GRADLE_CACHE_DIR
org.gradle.daemon=false
org.gradle.parallel=true
org.gradle.caching=true
org.gradle.configuration-cache=false
android.useAndroidX=true
android.enableJetifier=true
EOF

    if [ "$CIRCLECI" = "true" ]; then
        echo "CircleCI environment detected. Applying additional configurations..."
        
        # Enhanced Gradle options for CircleCI with Nix
        export GRADLE_OPTS="$GRADLE_OPTS -Dorg.gradle.unsafe.configuration-cache=false -Dorg.gradle.project.gradle.user.home=$GRADLE_CACHE_DIR"
        
        # Force Flutter to use our Gradle settings
        export FLUTTER_GRADLE_OPTS="$GRADLE_OPTS"
        
        echo "--- Global gradle.properties created ---"
        cat "$HOME/.gradle/gradle.properties"
        echo "GRADLE_USER_HOME: $GRADLE_USER_HOME"
        echo "GRADLE_OPTS: $GRADLE_OPTS"
        echo "TMPDIR: $TMPDIR"
        echo "----------------------------------------"
    fi

    # Create a local gradle wrapper properties to ensure consistent Gradle version
    mkdir -p "$WORK_DIR/gradle/wrapper"
    if [ ! -f "$WORK_DIR/gradle/wrapper/gradle-wrapper.properties" ]; then
        cat > "$WORK_DIR/gradle/wrapper/gradle-wrapper.properties" << EOF
distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-8.5-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
EOF
    fi

    export PATH=$NIX_TOOLCHAIN_BIN_PATH:$NIX_ANDROID_SDK/share/android-sdk/platform-tools:$NIX_ANDROID_SDK/share/android-sdk/tools:$NIX_ANDROID_SDK/share/android-sdk/tools/bin:$PATH

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
else
    # Non-Nix environment setup
    echo "=== Standard Environment Setup ==="
    
    # Set standard Gradle home for non-Nix environments
    if [ -z "$GRADLE_USER_HOME" ]; then
        export GRADLE_USER_HOME="$GRADLE_CACHE_DIR"
    fi
    
    # Basic Gradle options for non-Nix environments
    export GRADLE_OPTS="${GRADLE_OPTS:-} -Dgradle.user.home=$GRADLE_USER_HOME -Djava.io.tmpdir=$WORK_DIR/tmp"
    
    # Create gradle.properties for consistency
    mkdir -p "$HOME/.gradle"
    if [ ! -f "$HOME/.gradle/gradle.properties" ]; then
        cat > "$HOME/.gradle/gradle.properties" << EOF
systemProp.gradle.user.home=$GRADLE_USER_HOME
org.gradle.daemon=true
org.gradle.parallel=true
org.gradle.caching=true
android.useAndroidX=true
android.enableJetifier=true
EOF
    fi
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
echo "GRADLE_USER_HOME: $GRADLE_USER_HOME"
echo "PUB_CACHE: $PUB_CACHE"
echo "=================="

# Check if Flutter is present
if ! command -v flutter &> /dev/null; then
    echo "ERROR: Flutter command not found"
    exit 1
fi

# Verify Flutter can access writable directories
echo "Verifying Flutter configuration..."
flutter doctor -v

# Get dependencies
echo "Getting Flutter dependencies..."
flutter pub get

# Exec Flutter build
echo "Starting Flutter build..."
flutter build apk --release --verbose --split-per-abi

# Report build result
if [ $? -eq 0 ]; then
    echo "Built successfully!"
else
    echo "ERROR: build failed, please check output above for details"
    exit 1
fi
