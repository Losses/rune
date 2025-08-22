{ pkgs, masterPkgs, androidPkgs, androidSdk, rust-bin }:

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
  buildTools = androidPkgs."build-tools_${builtins.replaceStrings ["."] ["-"] buildToolsVersion}";
  aapt2Path = "${buildTools}/aapt2";

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
    masterPkgs.flutter
    android-studio
    androidSdk
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
    export LD_LIBRARY_PATH=$(nix-build '<nixpkgs>' -A wayland)/lib:${pkgs.fontconfig.lib}/lib:${pkgs.libxkbcommon}/lib:${pkgs.xorg.libX11}/lib:${pkgs.libGL}/lib:$LD_LIBRARY_PATH
    export PATH=$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
  '';
}
