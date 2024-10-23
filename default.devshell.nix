{ pkgs, masterPkgs, androidPkgs, androidSdk, rust-bin }:

let
  pinnedJDK = pkgs.jdk17;
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
    clang
    cmake
    protobuf_26
    pcre2
    mount
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
  ];

  env = {
    JAVA_HOME = "${pinnedJDK}";
    ANDROID_HOME = "${androidSdk}/share/android-sdk";
    RUST_BACKTRACE = 1;
    ANDROID_NDK_PATH = "${androidSdk}/share/android-sdk/ndk/ndk-27-1-12297006";
  };

  shellHook = ''
    alias ls=exa
    alias find=fd
    alias rinf='flutter pub run rinf'
    export LD_LIBRARY_PATH=$(nix-build '<nixpkgs>' -A wayland)/lib:${pkgs.fontconfig.lib}/lib:${pkgs.libxkbcommon}/lib:${pkgs.xorg.libX11}/lib:${pkgs.libGL}/lib:$LD_LIBRARY_PATH
    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidSdk}/share/android-sdk/build-tools/34.0.0/aapt2"
    export PATH=${androidSdk}/share/android-sdk/platform-tools:${androidSdk}/share/android-sdk/tools:${androidSdk}/share/android-sdk/tools/bin:$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
  '';
}