{ pkgs, masterPkgs, androidPkgs, androidSdk, rust-bin }:

let
  pinnedJDK = pkgs.jdk17;
  ndkVersion = "27.1.12297006";
  ndkRoot = "${androidSdk}/share/android-sdk/ndk/${ndkVersion}";
  toolchainPath = "${ndkRoot}/toolchains/llvm/prebuilt/linux-x86_64";
  sysrootPath = "${toolchainPath}/sysroot";
  toolchainBinPath = "${toolchainPath}/bin";
  cmakeToolchainFile = "${ndkRoot}/build/cmake/android.toolchain.cmake";
in
pkgs.mkShell {
  name = "Rune Development Shell";

  buildInputs = with pkgs; [
    (rust-bin.stable.latest.default.override {
      extensions = [ "rust-src" ];
      targets = [ "armv7-linux-androideabi" "aarch64-linux-android" "x86_64-linux-android" ];
    })
    yq
    openssl
    pkg-config
    stdenv
    clippy
    rust-analyzer
    rustup
    masterPkgs.flutter
    android-studio
    androidSdk
    pinnedJDK
    llvmPackages_18.clangUseLLVM
    cmake
    protobuf_26
    pcre2
    ninja
    unzip
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
    ANDROID_NDK_PATH = "${ndkRoot}";
    ANDROID_NDK_ROOT = "${ndkRoot}";
    CFLAGS = "-I${sysrootPath}/usr/include";
    CXXFLAGS = "-I${sysrootPath}/usr/include/c++/v1";
    BINDGEN_EXTRA_CLANG_ARGS = "--sysroot=${sysrootPath}";
    RUSTFLAGS = "-Clink-arg=--sysroot=${sysrootPath}";
    CMAKE_TOOLCHAIN_FILE = cmakeToolchainFile;
  };

  shellHook = ''
    echo "!! ANDROID_NDK_PATH: $ANDROID_NDK_PATH"
    echo "!! ANDROID_NDK_ROOT: $ANDROID_NDK_ROOT"
    echo "!! CMAKE_TOOLCHAIN_FILE: $CMAKE_TOOLCHAIN_FILE"
    echo "!! BINDGEN_EXTRA_CLANG_ARGS: $BINDGEN_EXTRA_CLANG_ARGS"
    flutter config --jdk-dir "${pinnedJDK}"
    alias ls=exa
    alias find=fd
    alias rinf='flutter pub run rinf'
    export LD_LIBRARY_PATH=$(nix-build '<nixpkgs>' -A wayland)/lib:${pkgs.fontconfig.lib}/lib:${pkgs.libxkbcommon}/lib:${pkgs.xorg.libX11}/lib:${pkgs.libGL}/lib:$LD_LIBRARY_PATH
    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdk}/share/android-sdk/build-tools/34.0.0/aapt2"
    export PATH=${toolchainBinPath}:${androidSdk}/share/android-sdk/platform-tools:${androidSdk}/share/android-sdk/tools:${androidSdk}/share/android-sdk/tools/bin:$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
    export LIBCLANG_PATH="${pkgs.libclang.lib}/lib"
    export PKG_CONFIG_SYSROOT_DIR="${sysrootPath}"
  '';
}
