# Flutter SDK for Windows (supports x64 and arm64 targets)
#
# Fetches the official portable Windows archive and unpacks it into the store.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "3.27.1"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  flutterArchive = fetchurl {
    url = "https://storage.googleapis.com/flutter_infra_release/releases/stable/windows/flutter_windows_${version}-stable.tar.zst";
    sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
  };

  package = derivation {
    name = "flutter-windows-${arch}-${version}";
    system = system;
    builder = "builtin:unpack";
    srcs = [ flutterArchive ];
  };

in rec {
  inherit version arch package;

  binPath = "${package}/bin";

  # Flags to pass to flutter build windows
  buildFlag = if arch == "arm64" then "--arm64" else "";

  env = {
    PUB_CACHE = "$env:LOCALAPPDATA\\Pub\\Cache";
  };

  setupHookPwsh = ''
    $env:PATH = "${binPath};" + $env:PATH
  '';
}
