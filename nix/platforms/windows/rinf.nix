# rinf CLI (rinf_cli crate) built from source as a Windows derivation.
#
# Replaces the CI "cargo install rinf_cli --locked --force" step: the rinf
# source .crate is hash-verified by fetchurl (fixed-output, network), and its
# 148 transitive dependencies are provided as a deterministic, hash-verified
# vendor tarball (hosted on Losses/rune).  A cmd.exe derivation then compiles
# the crate fully offline (--offline --frozen) against the store Rust toolchain
# and copies the single rinf.exe into $out/bin.
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

  # Deterministic vendor tarball: 148 crates under vendor/ plus
  # .cargo/config.toml that redirects crates-io to the vendored directory,
  # so the build needs no network.
  rinfVendor = fetchurl {
    url = "https://github.com/Losses/rune/releases/download/rinf-vendor-8.7.1/rinf-vendor-8.7.1.tar.gz";
    sha256 = "df94942846c68c984c7b592f2be1105732aa0003bef5b974099bd8eec173abf2";
  };

  # Step 1: unpack the .crate (a gzipped tar) AND the vendor tarball, so $out
  # holds rinf_cli-<version>/ (with its Cargo.lock), vendor/, and
  # .cargo/config.toml.  cargo run from $out then resolves deps offline.
  rinfSource = derivation {
    name = "rinf-source-${version}";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && tar.exe -xf %src% -C %out% && tar.exe -xf %vendorSrc% -C %out%" ];
    src = rinfCrate;
    vendorSrc = rinfVendor;
    PATH = "C:\\Windows\\System32";
  };

  # Step 2: build + install the rinf binary, fully offline from vendor/.
  # cd /d %src% lets cargo find .cargo/config.toml (vendored sources).
  package = derivation {
    name = "rinf-${version}";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && set CARGO_HOME=%TEMP%\\.cargo-rinf && cd /d %src% && ${rust.cargoExe} install --path %src%\\rinf_cli-${version} --locked --offline --frozen --root %out%" ];
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
