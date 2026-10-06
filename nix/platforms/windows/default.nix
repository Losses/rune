# Windows Platform Adapter
#
# Exposes the standard contract:
# - devShells.default: Complete MSVC + Rust + Flutter dev shell
# - packages.default: Rune Windows application package
#
# Can be evaluated through flake.nix or standalone by nova-nix:
#   nova-nix build nix/platforms/windows/default.nix

{ system ? "x86_64-windows"
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
    inherit fetchurl customMsvcRoot;
  };

  rust = import ./rust.nix {
    inherit fetchurl;
  };

  flutter = import ./flutter.nix {
    inherit fetchurl;
  };

  tools = import ./tools.nix {
    inherit fetchurl;
  };

  devshell = import ./devshell.nix {
    inherit msvc rust flutter tools;
  };

  runeWindows = derivation {
    name = "rune-windows";
    system = "x86_64-windows";
    builder = "builtin:unpack";
    srcs = [];
    passthru = {
      inherit msvc rust flutter tools devshell;
    };
  };

in {
  inherit msvc rust flutter tools;

  devShells = {
    default = devshell;
  };

  packages = {
    default = runeWindows;
  };
}
