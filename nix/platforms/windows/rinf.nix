# rinf CLI (rinf_cli crate) built from source as a Windows derivation.
#
# Deterministic offline build:
# - rinf_cli source crate is hash-verified via fetchurl from crates.io.
# - Transitive dependencies are parsed directly from rinf-Cargo.lock.
# - Each dependent crate is fetched from static.crates.io with its locked sha256 checksum.
# - The source and vendored crates are assembled, and cargo builds fully offline
#   (--offline --frozen) with the store Rust toolchain and host MSVC linker.

{ fetchurl ? (import <nix/fetchurl.nix>)
, rust
, msvc
, system ? "x86_64-windows"
, version ? "8.7.1"
}:

let
  lock = builtins.fromTOML (builtins.readFile ./rinf-Cargo.lock);
  depPackages = builtins.filter (p: p ? checksum) lock.package;

  rinfCrate = fetchurl {
    name = "crate-rinf_cli-${version}.tar.gz";
    url = "https://static.crates.io/crates/rinf_cli/${version}/download";
    sha256 = "1e82b79df638977ca905e91ede963b8849b7665cb8a445ab2b147dc76c890ab7";
  };

  mkDep = pkg: {
    inherit (pkg) name version checksum;
    tarball = fetchurl {
      name = "crate-${pkg.name}-${pkg.version}.tar.gz";
      url = "https://static.crates.io/crates/${pkg.name}/${pkg.version}/download";
      sha256 = pkg.checksum;
    };
  };

  deps = map mkDep depPackages;
  depTarballs = map (d: d.tarball) deps;

  # Convert Nix store path string into a clean Windows backslash path
  # so cmd.exe never misinterprets forward slashes as command switches.
  toWinPath = p: builtins.replaceStrings [ "/" ] [ "\\" ] (builtins.unsafeDiscardStringContext "${p}");

  # Build the Windows batch script that unpacks all vendored crates and
  # generates .cargo-checksum.json matching cargo's vendor layout.
  mkUnpackLine = d: let
    tb = toWinPath d.tarball;
    dir = "%out%\\vendor\\${d.name}-${d.version}";
  in "mkdir \"${dir}\"\r\ntar.exe -xf \"${tb}\" --strip-components=1 -C \"${dir}\"\r\necho {\"files\":{},\"package\":\"${d.checksum}\"}>\"${dir}\\.cargo-checksum.json\"";

  unpackScript = builtins.toFile "unpack-rinf-vendor.bat" (
    "@echo off\r\n" +
    "mkdir \"%out%\\vendor\"\r\n" +
    (builtins.concatStringsSep "\r\n" (map mkUnpackLine deps)) +
    "\r\n" +
    "mkdir \"%out%\\.cargo\"\r\n" +
    "echo [source.crates-io]>\"%out%\\.cargo\\config.toml\"\r\n" +
    "echo replace-with = \"vendored-sources\">>\"%out%\\.cargo\\config.toml\"\r\n" +
    "echo [source.vendored-sources]>>\"%out%\\.cargo\\config.toml\"\r\n" +
    "echo directory = \"vendor\">>\"%out%\\.cargo\\config.toml\"\r\n" +
    "exit /b 0\r\n"
  );

  # Step 1: unpack the rinf crate AND all vendored dependencies into $out,
  # setting up .cargo/config.toml so cargo resolves deps offline.
  rinfSource = derivation {
    name = "rinf-source-${version}";
    system = system;
    builder = "cmd.exe";
    args = [
      "/c"
      "mkdir %out% && tar.exe -xf %src% -C %out% && call %unpackScript%"
    ];
    src = rinfCrate;
    srcs = depTarballs;
    inherit unpackScript;
    PATH = "C:\\Windows\\System32";
  };

  # Step 2: build + install the rinf binary, fully offline from vendor/.
  package = derivation {
    name = "rinf-${version}";
    system = system;
    builder = "cmd.exe";
    args = [
      "/c"
      "mkdir %out%&&mkdir %out%\\.cargo-home&&set CARGO_HOME=%out%\\.cargo-home&&cd /d %src%&&${rust.cargoExe} install --path %src%\\rinf_cli-${version} --locked --offline --frozen --root %out%"
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
