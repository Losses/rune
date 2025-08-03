{ nixpkgs, rust-overlay, masterPkgs, system }:

let
  pkgsCross = nixpkgs.legacyPackages.x86_64-linux.pkgsCross.aarch64-multiplatform;
  rust-bin = rust-overlay.lib.mkRustBin { } pkgsCross.buildPackages;
in
pkgsCross.callPackage (
  {
    eza,
    mkShell,
    pkg-config,
    cmake,
    clang,
    binutils,
    qemu,
    openssl,
    stdenv,
    zstd,
    lmdb,
    sqlite,
    clippy,
    rust-analyzer,
    flutter,
    rustup,
    mount,
    protobuf_26,
    pcre2,
    ninja,
    unzip,
    fd,
    alsa-lib,
    libpulseaudio,
    pulseaudioFull,
    gtk3,
    fontconfig,
    mesa,
    libxkbcommon,
    libGL,
    wayland,
    gcc,
    xorg,
    libffi,
  }:
  mkShell {
    name = "Cross Shell that Combined Flutter and Rust Dev Shell";

    nativeBuildInputs = [
      eza
      (rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" ];
        targets = [ "aarch64-unknown-linux-gnu" ];
      })
      pkg-config
      clang
      cmake
      binutils
      flutter
      clippy
      rust-analyzer
      rustup
      mount
      protobuf_26
      pcre2
      ninja
      unzip
    ];

    depsBuildBuild = [ qemu ];

    buildInputs = [
      fd
      libpulseaudio
      pulseaudioFull
      gtk3
      fontconfig
      mesa
      libxkbcommon
      xorg.libX11
      libGL
      wayland
      zstd
      lmdb
      sqlite
      openssl
      alsa-lib
      libffi
    ];

    env = {
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${stdenv.cc.targetPrefix}cc";
      CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER = "qemu-aarch64";
      RUST_BACKTRACE = 1;
      PKG_CONFIG_ALLOW_CROSS = 1;
      ZSTD_SYS_USE_PKG_CONFIG = 1;
      LIBSQLITE3_SYS_USE_PKG_CONFIG = 1;
    };

    shellHook = ''
      alias ls=exa
      alias find=fd
      alias build='flutter-elinux build elinux --target-arch=arm64'
      export PATH=$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
      export LDFLAGS="-L${stdenv.cc.cc.lib}/lib -L${wayland}/lib $LDFLAGS"
      export PKG_CONFIG_PATH=${zstd.dev}/lib/pkgconfig:${lmdb.dev}/lib/pkgconfig:${sqlite.dev}/lib/pkgconfig:${openssl.dev}/lib/pkgconfig:${alsa-lib.dev}/lib/pkgconfig:${libffi.dev}/lib/pkgconfig:${wayland.dev}/lib/pkgconfig
      export CC="${stdenv.cc.targetPrefix}clang"
      export CXX="${stdenv.cc.targetPrefix}clang++"

      echo "${libffi}"
      echo "${libffi.dev}"
      echo "Using CC: $CC"
      echo "Using CXX: $CXX"
    '';
  }
) { };
