# Windows Platform Adapter
# Supports both x86_64-windows (x64) and aarch64-windows (arm64).
#
# Exposes the standard contract:
# - devShells.default: Complete MSVC + Rust + Flutter dev shell
# - packages.default: Rune Windows application package
#
# Can be evaluated through flake.nix or standalone by nova-nix:
#   nova-nix eval nix/platforms/windows/default.nix

{ system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
, inputs ? {}
, customMsvcRoot ? null
}:

let
  # Use built-in fetchurl if standalone, or fallback to nixpkgs fetchurl if available
  fetchurl = if inputs ? nixpkgs then
    (import inputs.nixpkgs { inherit system; }).fetchurl
  else
    (import <nix/fetchurl.nix>);

  msvc = import ./msvc.nix {
    inherit customMsvcRoot system arch;
  };

  rust = import ./rust.nix {
    inherit fetchurl system arch;
  };

  flutter = import ./flutter.nix {
    inherit fetchurl system arch;
  };

  tools = import ./tools.nix {
    inherit fetchurl system arch;
  };

  innosetup = import ./innosetup.nix {
    inherit fetchurl system arch;
  };

  devshell = import ./devshell.nix {
    inherit msvc rust flutter tools system arch;
  };

  runeWindows = import ./package.nix {
    inherit msvc rust flutter tools system arch;
  };

in {
  inherit msvc rust flutter tools innosetup arch system;

  devShells = {
    default = devshell;
  };

  packages = {
    default = runeWindows;
  };
}
