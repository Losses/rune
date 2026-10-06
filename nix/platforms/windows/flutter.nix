# Flutter SDK for Windows
#
# Fetches the official portable Windows archive and unpacks it into the store.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "3.27.1"
}:

let
  flutterArchive = fetchurl {
    url = "https://storage.googleapis.com/flutter_infra_release/releases/stable/windows/flutter_windows_${version}-stable.tar.zst";
    sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
  };

  package = derivation {
    name = "flutter-windows-${version}";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [ flutterArchive ];
  };

in rec {
  inherit version package;

  binPath = "${package}/bin";

  env = {
    PUB_CACHE = "$env:LOCALAPPDATA\\Pub\\Cache";
  };

  setupHookPwsh = ''
    $env:PATH = "${binPath};" + $env:PATH
  '';
}
