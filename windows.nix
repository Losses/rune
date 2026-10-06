# Direct entry point for Windows native Nix / nova-nix
# Usage:
#   nova-nix eval windows.nix
#   nova-nix build windows.nix

let
  platform = import ./nix/platforms/windows/default.nix { };
in
platform.packages.default // {
  inherit (platform) devShells packages msvc rust flutter tools;
}
