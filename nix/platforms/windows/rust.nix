# Official Rust Toolchain targeting Windows (x86_64 or aarch64 MSVC)
#
# Supports:
# - x86_64-pc-windows-msvc (x64)
# - aarch64-pc-windows-msvc (Windows on ARM)

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "1.98.1"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  target = if arch == "arm64" then "aarch64-pc-windows-msvc" else "x86_64-pc-windows-msvc";

  rustArchive = fetchurl {
    url = "https://static.rust-lang.org/dist/rust-${version}-${target}.tar.zst";
    sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
  };

  package = derivation {
    name = "rust-${version}-${target}";
    system = system;
    builder = "builtin:unpack";
    srcs = [ rustArchive ];
  };

in rec {
  inherit version target arch package;

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
