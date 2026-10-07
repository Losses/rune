# Flutter SDK for Windows (supports x64 and arm64 targets)
# Sourced directly from official Google Flutter Infra matching Scoop Extras bucket.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "3.47.6"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  flutterManifests = {
    "3.47.6" = {
      archive = "flutter_windows_3.47.6-stable.zip";
      sha256 = "a01bb0d26de91bc23c97cd9ccfaad281a612fb8304213fdd5df1119a09404796";
    };
    "3.27.4" = {
      archive = "flutter_windows_3.27.4-stable.zip";
      sha256 = "1141d3edb64c454273feac88f31f84945a1f3309a72c58aa9a5f2bb2b0fc8db3";
    };
    "3.27.1" = {
      archive = "flutter_windows_3.27.1-stable.zip";
      sha256 = "7e72b71b3570a117c6070a5935bcf3ccd0254a6d63e5eff99d4bc4ddb5006ca9";
    };
  };

  selected = flutterManifests.${version};

  flutterArchive = fetchurl {
    url = "https://storage.googleapis.com/flutter_infra_release/releases/stable/windows/${selected.archive}";
    sha256 = selected.sha256;
  };

  package = derivation {
    name = "flutter-windows-${arch}-${version}";
    system = system;
    builder = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";
    args = [ "-NoProfile" "-ExecutionPolicy" "Bypass" "-File" ./flutter-build.ps1 ];
    src = flutterArchive;
    launcher = ./flutter-launcher.ps1;
    PATH = "C:\\Windows\\System32;C:\\Windows\\System32\\WindowsPowerShell\\v1.0";
  };

in rec {
  inherit version arch package;

  binPath = "${package}/flutter/bin";

  # Flags to pass to flutter build windows
  buildFlag = ""; # Flutter 'build windows' targets the host arch only; --arm64 is not a valid flag

  env = {
    PUB_CACHE = "$env:LOCALAPPDATA\\Pub\\Cache";
  };

  setupHookPwsh = ''
    $env:PATH = "${binPath};" + $env:PATH
  '';
}
