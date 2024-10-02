{ pkgs, masterPkgs, androidCustomPackage, pinnedJDK, rust-bin }:

pkgs.mkShell {
  name = "Rune Development Shell";

  buildInputs = with pkgs; [
    (rust-bin.stable.latest.default.override {
      extensions = [ "rust-src" ];
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
    androidCustomPackage
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
    ANDROID_HOME = "${androidCustomPackage}/share/android-sdk";
    GRADLE_USER_HOME = "/home/specx/.gradle";
    RUST_BACKTRACE = 1;
  };

  shellHook = ''
    alias ls=exa
    alias find=fd
    alias rinf='flutter pub run rinf'
    export LD_LIBRARY_PATH=$(nix-build '<nixpkgs>' -A wayland)/lib:${pkgs.fontconfig.lib}/lib:${pkgs.libxkbcommon}/lib:${pkgs.xorg.libX11}/lib:${pkgs.libGL}/lib:$LD_LIBRARY_PATH
    export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidCustomPackage}/share/android-sdk/build-tools/34.0.0/aapt2"
    export PATH=${androidCustomPackage}/share/android-sdk/platform-tools:${androidCustomPackage}/share/android-sdk/tools:${androidCustomPackage}/share/android-sdk/tools/bin:$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
  '';
}
