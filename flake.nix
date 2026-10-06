{
  description = "A combined Flutter and Rust devShell";

  inputs = {
    nixpkgs = {
      url = "github:NixOS/nixpkgs/nixos-unstable";
    };
    master-nixpkgs = {
      url = "github:NixOS/nixpkgs/master";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
    };
    flake-compat = {
      url = "github:edolstra/flake-compat";
      flake = false;
    };
    android-nixpkgs = {
      url = "github:tadfisher/android-nixpkgs";
    };
  };

  outputs = { self, nixpkgs, master-nixpkgs, rust-overlay, flake-utils, flake-compat, android-nixpkgs, ... }@inputs:
    let
      supportedSystems = flake-utils.lib.defaultSystems ++ [ "x86_64-windows" ];

      platformAdapters = {
        "x86_64-windows" = import ./nix/platforms/windows;
        default = import ./nix/platforms/posix.nix;
      };

      getAdapter = system:
        platformAdapters.${system} or platformAdapters.default;
    in
    flake-utils.lib.eachSystem supportedSystems (system:
      let
        adapter = (getAdapter system) {
          inherit system inputs;
        };
      in {
        inherit (adapter) devShells packages;
      }
    );
}
