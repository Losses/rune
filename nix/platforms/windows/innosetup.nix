# Inno Setup compiler (ISCC.exe) for Windows
# Sourced from official jrsoftware/issrc GitHub releases matching Scoop Extras bucket.

{ fetchurl ? (import <nix/fetchurl.nix>)
, version ? "6.7.3"
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  # Verified SHA256 checksums from Scoop Extras (bucket/inno-setup.json).
  innoManifests = {
    "6.7.3" = {
      url = "https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe";
      sha256 = "9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732";
    };
  };

  selected = innoManifests.${version};
  innoArchive = fetchurl {
    inherit (selected) url sha256;
  };

  # The package is directly the fetchurl output — the verified installer
  # .exe file itself, already resident in the Nix store.  Like the Rust MSI,
  # silent installation (/VERYSILENT) runs in the CI workflow step OUTSIDE
  # the sandbox; the Inno Setup installer is a GUI app that cannot run
  # inside the nova-nix build sandbox.
  package = innoArchive;

in rec {
  inherit version arch package;
}
