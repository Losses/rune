# Official Rust Toolchain targeting Windows (x86_64 or aarch64 MSVC)
# Sourced directly from official static.rust-lang.org distributions.
#
# Supports:
# - x86_64-pc-windows-msvc (x64)
# - aarch64-pc-windows-msvc (Windows on ARM)
#
# Distribution format: the standalone .tar.xz (NOT the MSI).  The tarball
# extracts in the nova-nix sandbox via cmd.exe + tar.exe (same pattern as
# flutter.nix), so the toolchain lands fully inside the Nix store instead of
# being pulled apart by msiexec in a CI step.
#
# The standalone tarball ships rustc and its target std in SEPARATE
# component dirs (rustc/ and rust-std-<target>/); we xcopy the std tree
# into rustc/ so that cargo's sysroot resolution finds rlibs.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "1.99.0"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  target = if arch == "arm64" then "aarch64-pc-windows-msvc" else "x86_64-pc-windows-msvc";

  # Verified SHA256 checksums from static.rust-lang.org (standalone tar.xz).
  rustManifests = {
    "1.99.0" = {
      "x86_64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.99.0-x86_64-pc-windows-msvc.tar.xz";
        sha256 = "209f916c04cd7ed1938a4592e8adc1b1017adada67dc8f8a0a92bca9c4963966";
      };
      "aarch64-pc-windows-msvc" = {
        url = "https://static.rust-lang.org/dist/rust-1.99.0-aarch64-pc-windows-msvc.tar.xz";
        sha256 = "d877f4b3727eb2f6702da4b083a9701c9f4135c6d0c1bc03d516534bec89c8c0";
      };
    };
  };

  selected = rustManifests.${version}.${target};
  rustArchive = fetchurl {
    inherit (selected) url sha256;
  };

  # Extract the tarball into the store.  --strip-components=1 drops the
  # leading rust-1.99.0-<target>/ so $out holds cargo/, rustc/,
  # rust-std-<target>/, ...
  #
  # After extraction we merge the target std libs into rustc/ (xcopy
  # /E sinks rust-std-<target>/* into rustc/), reproducing the flat
  # layout that rustup's install.sh creates.  Without this merge rustc
  # cannot find its target std rlibs.
  # Proven single-line cmd.exe pattern (matching tools.nix & flutter.nix):
  # && chaining avoids multi-line issues; no quotes around env vars
  # (store paths have no spaces).
  package = derivation {
    name = "rust-${target}-${version}";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && tar.exe -xf %src% --strip-components=1 -C %out% && xcopy /E /I /Y %out%\\rust-std-${target} %out%\\rustc" ];
    src = rustArchive;
    PATH = "C:\\Windows\\System32";
  };

in rec {
  inherit version target arch package;

  # cargo and rustc live in separate top-level dirs after extraction; both
  # must be on PATH for cargo to locate rustc.
  rustcBin = "${package}/rustc/bin";
  cargoBin = "${package}/cargo/bin";

  cargoExe = "${cargoBin}/cargo.exe";
  rustcExe = "${rustcBin}/rustc.exe";

  # Semicolon-joined PATH fragment (back-compat with devshell.nix/package.nix,
  # which interpolate rust.binPath into a single PATH string).
  binPath = "${cargoBin};${rustcBin}";

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

