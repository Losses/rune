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
      in {
        devShells.default = pkgs.mkShell {
          name = "Combined Flutter and Rust Dev Shell";
          buildInputs = with pkgs; [
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [ "rust-src" "cargo" "rustc" ];
            })
            expidusPkgs.flutter
            android-studio
            openssl
            pkg-config
            eza
            fd
            alsa-lib
            libpulseaudio
            pulseaudioFull
            clippy
            rust-analyzer
            rustup
          ] ++ [
            gtk3
            pinnedJDK
            androidCustomPackage
            protobuf_26
            pcre2
            mount
            ninja
            clang
            cmake
            libstdcxx5
          ];

          RUST_SRC_PATH = "${pkgs.rust-bin.stable.latest.default.override {
            extensions = [ "rust-src" ];
          }}/lib/rustlib/src/rust/library";

          shellHook = ''
            alias ls=exa
            alias find=fd
            alias rinf='flutter pub run rinf'
            export RUST_BACKTRACE=1
            export JAVA_HOME=${pinnedJDK}
            export ANDROID_HOME=${androidCustomPackage}/share/android-sdk
            export GRADLE_USER_HOME=/home/specx/.gradle
            export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=${androidCustomPackage}/share/android-sdk/build-tools/34.0.0/aapt2"
            export PATH=${androidCustomPackage}/share/android-sdk/platform-tools:${androidCustomPackage}/share/android-sdk/tools:${androidCustomPackage}/share/android-sdk/tools/bin:$HOME/.cargo/bin:$HOME/.pub-cache/bin:$PATH
          '';
        };
      }
    );
}
