{ system, inputs }:

let
  inherit (inputs) nixpkgs master-nixpkgs rust-overlay android-nixpkgs;

  overlays = import ../overlays.nix { inherit rust-overlay; };
  pkgs = import nixpkgs {
    inherit system overlays;
    config = {
      allowUnfree = true;
      android_sdk = {
        accept_license = true;
      };
    };
  };

  prebuiltOpenSSL = pkgs.stdenv.mkDerivation rec {
    pname = "openssl-android-prebuilt";
    version = "3.4.0";

    src_arm64 = pkgs.fetchurl {
      url = "https://github.com/217heidai/openssl_for_android/releases/download/3.4.0/OpenSSL_3.4.0_arm64-v8a.tar.gz";
      hash = "sha256-s7dm7Wkvfwzs7X+QQgyd97qEBn1Fw4tamywN8kMEZNA=";
    };
    src_armv7 = pkgs.fetchurl {
      url = "https://github.com/217heidai/openssl_for_android/releases/download/3.4.0/OpenSSL_3.4.0_armeabi-v7a.tar.gz";
      hash = "sha256-7Yrz7YEEcpJncYO6H1T5nUW2BqSWOPv2OlQZdmbeqeo=";
    };
    src_x86 = pkgs.fetchurl {
      url = "https://github.com/217heidai/openssl_for_android/releases/download/3.4.0/OpenSSL_3.4.0_x86.tar.gz";
      hash = "sha256-LkxBgHjeHOXoszpyKx7JTen3JRYDPDgNi9tAXcVqoJU=";
    };
    src_x86_64 = pkgs.fetchurl {
      url = "https://github.com/217heidai/openssl_for_android/releases/download/3.4.0/OpenSSL_3.4.0_x86_64.tar.gz";
      hash = "sha256-PFBe28qXM0PLxiH5GfCJZjSEoMzyCP1gZ8dB+L/3/jk=";
    };

    srcs = [ src_arm64 src_armv7 src_x86 src_x86_64 ];
    sourceRoot = ".";

    installPhase = ''
      mkdir -p $out/arm64-v8a $out/armeabi-v7a $out/x86 $out/x86_64
      tar -xzf ${src_arm64} -C $out/arm64-v8a --strip-components=1
      tar -xzf ${src_armv7} -C $out/armeabi-v7a --strip-components=1
      tar -xzf ${src_x86} -C $out/x86 --strip-components=1
      tar -xzf ${src_x86_64} -C $out/x86_64 --strip-components=1
    '';
  };

  masterPkgs = import master-nixpkgs {
    inherit system;
  };
  
  androidPkgs = import android-nixpkgs {
    inherit system;
  };

  androidSdk = android-nixpkgs.sdk.${system} (
    sdkPkgs: with sdkPkgs; [
      cmdline-tools-latest
      build-tools-30-0-3
      build-tools-33-0-2
      build-tools-34-0-0
      build-tools-35-0-0
      build-tools-36-0-0
      platform-tools
      emulator
      platforms-android-28
      platforms-android-29
      platforms-android-30
      platforms-android-31
      platforms-android-32
      platforms-android-33
      platforms-android-34
      platforms-android-35
      platforms-android-36
      ndk-27-1-12297006
      system-images-android-34-default-x86-64
    ]
  );

  rust-bin = rust-overlay.lib.mkRustBin { } pkgs.buildPackages;

in {
  devShells = {
    default = import ../devshells/default.devshell.nix {
      inherit pkgs masterPkgs androidSdk androidPkgs rust-bin prebuiltOpenSSL;
    };

    cross = import ../devshells/cross.devshell.nix {
      inherit nixpkgs rust-overlay masterPkgs system;
    };
  };

  packages = {
    default = pkgs.callPackage ../packages/rune.nix { };
  };
}
