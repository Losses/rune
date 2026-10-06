# Official Rust Toolchain targeting x86_64-pc-windows-msvc
#
# Fetched directly as a store path to ensure pure, reproducible builds
# without requiring ambient rustup or administrator permissions.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "1.98.1"
}:

let
  target = "x86_64-pc-windows-msvc";

  # Rust toolchain archive from rust-lang static CDN
  rustArchive = fetchurl {
    url = "https://static.rust-lang.org/dist/rust-${version}-${target}.tar.zst";
    # Fallback to .tar.gz / .zip if needed depending on CDN format
    sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
  };

  package = derivation {
    name = "rust-${version}-${target}";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [ rustArchive ];
  };

in rec {
  inherit version target package;

  binPath = "${package}/bin";

  env = {
    RUST_BACKTRACE = "1";
    CARGO_BUILD_TARGET = target;
  };

  setupHookPwsh = ''
    $env:PATH = "${binPath};" + $env:PATH
    $env:RUST_BACKTRACE = "1"
    $env:CARGO_BUILD_TARGET = "${target}"
  '';
}
