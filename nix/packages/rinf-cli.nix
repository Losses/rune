# The rinf code generator, pinned to the version the app's `rinf` Dart package
# and Rust crate are written for. The prebuilt binary from crates.io
# (`cargo-binstall`) has a hard-coded dynamic loader and cannot run on NixOS,
# so it is built from source.
{ pkgs }:
pkgs.rustPlatform.buildRustPackage rec {
  pname = "rinf_cli";
  version = "8.7.1";

  src = pkgs.fetchCrate {
    inherit pname version;
    hash = "sha256-MDiWD2CevkJHRTmnBWpvHra3i2985nO5mJ9uOVK55l8=";
  };

  cargoHash = "sha256-RIccbZFfMcxhmMuN72YQbAbRnDrpz/i4I7Zb8WngXT4=";
}
