# Official Rust Toolchain targeting Windows (x86_64 or aarch64 MSVC)
# Sourced directly from official static.rust-lang.org distributions.
#
# Supports:
# - x86_64-pc-windows-msvc (x64)
# - aarch64-pc-windows-msvc (Windows on ARM)

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "1.99.0"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  target = if arch == "arm64" then "aarch64-pc-windows-msvc" else "x86_64-pc-windows-msvc";

  # Verified SHA256 checksums from static.rust-lang.org matching Scoop Main
  rustManifests = {
    "1.99.0" = {
      "x86_64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.99.0-x86_64-pc-windows-msvc.msi";
        sha256 = "0ccecc0d77722cf4ab3288ecdcf541530bec129b530f91618cbdd9e3a0dbe7fd";
      };
      "aarch64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.99.0-aarch64-pc-windows-msvc.msi";
        sha256 = "c936b4067ed3b53f7ea8fc8044f7d66ed17dfa3675005976a452ab93a1110052";
      };
    };
    "1.98.1" = {
      "x86_64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.98.1-x86_64-pc-windows-msvc.msi";
        sha256 = "346bea0c3076a33e291624b3d7e71bb3cf661422fadaf643612de641b3e7599a";
      };
      "aarch64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.98.1-aarch64-pc-windows-msvc.msi";
        sha256 = "6bcda8cbf1151a0854e87c51080c7ff2d77356dd87ff4b2157b93aa870093ae6";
      };
    };
  };

  selected = rustManifests.${version}.${target};
  rustArchive = fetchurl {
    inherit (selected) url sha256;
  };

  package = derivation {
    name = "rust-${version}-${target}";
    system = system;
    builder = "cmd.exe";
    args = [
      "/c"
      "mkdir %out% && msiexec /a %src% /qn TARGETDIR=%out%"
    ];
    src = rustArchive;
    PATH = "C:\\Windows\\System32";
  };

in rec {
  inherit version target arch package;

  binPath = "${package}/Rust/bin";

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
