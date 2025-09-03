{ pkgs, masterPkgs, androidPkgs, androidSdk, rust-bin,
  prebuiltOpenSSL
}:

let
  pinnedJDK = pkgs.jdk17;

  # NDK setup
  ndkVersion = "27.1.12297006";
  ndkRoot = "${androidSdk}/share/android-sdk/ndk/${ndkVersion}";
  toolchainPath = "${ndkRoot}/toolchains/llvm/prebuilt/linux-x86_64";
  sysrootPath = "${toolchainPath}/sysroot";
  toolchainBinPath = "${toolchainPath}/bin";
  cmakeToolchainFile = "${ndkRoot}/build/cmake/android.toolchain.cmake";

  # Android Build Tools setup for aapt2
  buildToolsVersion = "34.0.0";
  aapt2Path = "${androidSdk}/share/android-sdk/build-tools/${buildToolsVersion}/aapt2";

in
pkgs.mkShell {
  name = "Rune Development Shell";

  buildInputs = with pkgs; [
    (rust-bin.stable.latest.default.override {
      extensions = [ "rust-src" ];
      targets = [ "armv7-linux-androideabi" "aarch64-linux-android" "i686-linux-android" "x86_64-linux-android" ];
    })
    yq
    openssl
    pkg-config
    stdenv
    clippy
    rust-analyzer
    rustup
    flutter
    android-studio
    pinnedJDK
    clang
    cmake
    pcre2
    ninja
    unzip
    curl
    wayland
    eza
    fd
    gtk3
    libpulseaudio
    pulseaudioFull
    fontconfig
    mesa
    libxkbcommon
    pkgs.xorg.libX11
    libGL
    alsa-lib.dev
    wayland.dev
    zstd.dev
    lmdb.dev
    sqlite.dev
    util-linux.dev
    libsysprof-capture
    libayatana-appindicator
    libnotify.dev
  ];

  env = {
    JAVA_HOME = "${pinnedJDK}";
    ANDROID_HOME = "${androidSdk}/share/android-sdk";
    RUST_BACKTRACE = 1;
    ANDROID_NDK_PATH = ndkRoot;
    NIX_NIX_DEV_SHELL = "true";
    NIX_ANDROID_NDK_ROOT = ndkRoot;
    NIX_CFLAGS = "-I${sysrootPath}/usr/include";
    NIX_CXXFLAGS = "-I${sysrootPath}/usr/include/c++/v1";
    NIX_BINDGEN_EXTRA_CLANG_ARGS = "--sysroot=${sysrootPath}";
    NIX_RUSTFLAGS = "-Clink-arg=--sysroot=${sysrootPath}";
    NIX_CMAKE_TOOLCHAIN_FILE = cmakeToolchainFile;
    NIX_TOOLCHAIN_BIN_PATH = toolchainBinPath;
    NIX_ANDROID_SDK = androidSdk;
    NIX_PINNED_JDK = pinnedJDK;
    NIX_GRADLE_OPTS = "-Dorg.gradle.project.android.aapt2FromMavenOverride=${aapt2Path}";
  };

  shellHook = ''
    alias ls=exa
    alias find=fd
    flutter config --jdk-dir "${pinnedJDK}"
    export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath (with pkgs; [ wayland fontconfig libxkbcommon xorg.libX11 libGL ])}:$LD_LIBRARY_PATH
    export PATH=$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH

    setup_android_env() {
      echo "Setting up environment for Android cross-compilation..."
      export _NATIVE_PATH=$PATH
      export ANDROID_NDK_ROOT="${ndkRoot}"
      export ANDROID_NDK_PATH="${ndkRoot}"
      export CMAKE_TOOLCHAIN_FILE="${cmakeToolchainFile}"
      export CFLAGS="-I${sysrootPath}/usr/include"
      export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=${sysrootPath}"
      export RUSTFLAGS="-Clink-arg=--sysroot=${sysrootPath}"
      export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${aapt2Path}"
      export PKG_CONFIG_PATH="${toolchainBinPath}"
      export PKG_CONFIG_SYSROOT_DIR="${sysrootPath}"
      export _JAVA_OPTIONS="-Dorg.gradle.projectcachedir=$(mktemp -d)"

      # Point to the correct subdirectories within the fetched archive
      # This now perfectly mirrors the logic from your build.sh script.
      export ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR="${prebuiltOpenSSL}/armeabi-v7a"
      export AARCH64_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/arm64-v8a"
      export I686_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/x86"
      export X86_64_LINUX_ANDROID_OPENSSL_DIR="${prebuiltOpenSSL}/x86_64"

      export PATH="${toolchainBinPath}:${androidSdk}/share/android-sdk/platform-tools:${androidSdk}/share/android-sdk/tools:${androidSdk}/share/android-sdk/tools/bin:$PATH"
      echo "Android environment is ready."
    }

    teardown_android_env() {
      echo "Restoring native build environment..."
      if [ -n "$_NATIVE_PATH" ]; then export PATH=$_NATIVE_PATH && unset _NATIVE_PATH; fi
      unset ANDROID_NDK_ROOT ANDROID_NDK_PATH CMAKE_TOOLCHAIN_FILE CFLAGS BINDGEN_EXTRA_CLANG_ARGS RUSTFLAGS GRADLE_OPTS PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR
      unset ARMV7_LINUX_ANDROIDEABI_OPENSSL_DIR AARCH64_LINUX_ANDROID_OPENSSL_DIR I686_LINUX_ANDROID_OPENSSL_DIR X86_64_LINUX_ANDROID_OPENSSL_DIR
      echo "Native environment restored."
    }

    echo "--------------------------------------------------------"
    echo "Nix shell is ready for native (x86) development."
    echo "To build for Android, run: setup_android_env"
    echo "To return to native mode, run: teardown_android_env"
    echo "--------------------------------------------------------"
  '';
}
