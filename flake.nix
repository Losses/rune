{
  description = "A combined Flutter and Rust devShell";

  inputs = {
    nixpkgs = {
      url = "github:NixOS/nixpkgs/nixos-unstable";
    };
    master-nixpkgs = {
      url = "github:NixOS/nixpkgs/master";
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
  };

  outputs = { self, nixpkgs, master-nixpkgs, rust-overlay, flake-utils, flake-compat, android-nixpkgs, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = import ./overlays.nix { inherit rust-overlay; };
        pkgs = import nixpkgs {
          inherit system overlays;
          config = {
            allowUnfree = true;
            android_sdk = {
              accept_license = true;
            };
          };
        };

        masterPkgs = import master-nixpkgs {
          inherit system;
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
        devShells.default = import ./default.devshell.nix {
          inherit pkgs masterPkgs androidCustomPackage pinnedJDK rust-bin;
        };

        devShells.cross = import ./cross.devshell.nix {
          inherit nixpkgs rust-overlay masterPkgs system;
        };

        packages.default = pkgs.callPackage ./rune.nix {
          inherit (masterPkgs) lib jq stdenv fetchzip makeDesktopItem moreutils cargo rustPlatform rustc alsa-lib lmdb flutter324 protobuf_26 protoc-gen-prost buildDartApplication dart;
        };
      }
    );
}
