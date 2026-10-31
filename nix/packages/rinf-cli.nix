# The rinf code generator, pinned to the version the app's `rinf` Dart package
# and Rust crate are written for. The prebuilt binary from crates.io
# (`cargo-binstall`) has a hard-coded dynamic loader and cannot run on NixOS,
# so it is built from source.
{ pkgs }:
pkgs.rustPlatform.buildRustPackage rec {
  pname = "rinf_cli";
  version = "8.11.0";

  src = pkgs.fetchCrate {
    inherit pname version;
    hash = "sha256-Vzfmjewvl6gt06pmjE5ZZ9D54MzusAmXGkRqZSclujo=";
  };

  cargoHash = "sha256-POyL32SwjCjOaYKbsYmdhsX8ViRa00aQQaSuoB9+XW8=";
}
