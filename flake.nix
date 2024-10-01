{
  description = "A combined Flutter and Rust devShell";

  inputs = {
    nixpkgs = {
      url = "github:NixOS/nixpkgs/nixos-unstable";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    android-nixpkgs = {
      url = "github:tadfisher/android-nixpkgs";
    };
    expidus-nixpkgs = {
      url = "github:ExpidusOS/nixpkgs/feat/flutter-3-24";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, flake-compat, android-nixpkgs, expidus-nixpkgs, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
            android_sdk = {
              accept_license = true;
            };
          };
        };

        expidusPkgs = import expidus-nixpkgs { inherit system; };

        androidEnvCustomPackage = pkgs.androidenv.composeAndroidPackages {
          toolsVersion = "26.1.1";
          platformToolsVersion = "34.0.5";
          buildToolsVersions = [ "30.0.3" "34.0.0" ];
          includeEmulator = true;
          emulatorVersion = "34.1.9";
          platformVersions = [ "28" "29" "30" "31" "32" "33" "34" ];
          includeSources = false;
          includeSystemImages = false;
          systemImageTypes = [ "google_apis_playstore" ];
          abiVersions = [ "armeabi-v7a" "arm64-v8a" ];
          cmakeVersions = [ "3.10.2" ];
          includeNDK = true;
          ndkVersions = [ "22.0.7026061" ];
          useGoogleAPIs = false;
          useGoogleTVAddOns = false;
        };

        androidCustomPackage = android-nixpkgs.sdk.${system} (
          sdkPkgs: with sdkPkgs; [
            cmdline-tools-latest
            build-tools-30-0-3
            build-tools-33-0-2
            build-tools-34-0-0
            platform-tools
            emulator
            platforms-android-28
            platforms-android-29
            platforms-android-30
            platforms-android-31
            platforms-android-32
            platforms-android-33
            platforms-android-34
          ]
        );

        pinnedJDK = pkgs.jdk17;

        rust-bin = rust-overlay.lib.mkRustBin { } pkgs.buildPackages;
      in {
        # Development environmet for general purpose
        devShells.default = pkgs.mkShell {
          name = "Combined Flutter and Rust Dev Shell";

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
            expidusPkgs.flutter
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
        };


        # Development environmet flutter-elinux
        devShells.cross = let
          pkgsCross = nixpkgs.legacyPackages.x86_64-linux.pkgsCross.aarch64-multiplatform;
          rust-bin = rust-overlay.lib.mkRustBin { } pkgsCross.buildPackages;

          expidusPkgs = import expidus-nixpkgs { inherit system; };
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
              expidusPkgs.flutter
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
              alias rinf='flutter pub run rinf'
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
      }
    );
}
