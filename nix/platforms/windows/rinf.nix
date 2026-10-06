# rinf CLI (rinf_cli crate) built from source as a Windows derivation.
#
# Replaces the CI "cargo install rinf_cli --locked --force" step: the rinf
# source .crate is hash-verified by fetchurl (fixed-output, network), then a
# cmd.exe derivation compiles it with the store Rust toolchain and copies the
# single rinf.exe into $out/bin.
#
# MSVC env vars are threaded through exactly as package.nix does for the
# hub.dll build, so the sandbox inherits the host Visual Studio linker (link.exe)
# that cargo needs to link the final binary.

{ fetchurl ? (import <nix/fetchurl.nix>)
, rust
, msvc
, system ? "x86_64-windows"
, version ? "8.7.1"
}:

let
  rinfCrate = fetchurl {
    url = "https://crates.io/api/v1/crates/rinf_cli/${version}/download";
    sha256 = "1e82b79df638977ca905e91ede963b8849b7665cb8a445ab2b147dc76c890ab7";
  };

  # Step 1: unpack the .crate (a gzipped tar) so cargo --path can consume it.
  rinfSource = derivation {
    name = "rinf-source-${version}";
    system = system;
    builder = "cmd.exe";
    args = [
      "/c"
      ''
        mkdir "%out%"
        tar.exe -xf "%src%" -C "%out%"
      ''
    ];
    src = rinfCrate;
    PATH = "C:\\Windows\\System32";
  };

  # Step 2: build + install the rinf binary.
  package = derivation {
    name = "rinf-${version}";
    system = system;
    builder = "cmd.exe";
    args = [
      "/c"
      ''
        mkdir "%out%"
        set "CARGO_HOME=%TEMP%\.cargo-rinf"
        "${rust.cargoExe}" install --path "%src%\rinf_cli-${version}" --locked --root "%out%"
      ''
    ];
    src = rinfSource;
    PATH = "C:\\Windows\\System32;${msvc.env.PATH};${rust.binPath}";
    VCINSTALLDIR = msvc.env.VCINSTALLDIR;
    WindowsSdkDir = msvc.env.WindowsSdkDir;
    INCLUDE = msvc.env.INCLUDE;
    LIB = msvc.env.LIB;
  };

in {
  inherit version package;
  binPath = "${package}/bin";
  exe = "${package}/bin/rinf.exe";
}