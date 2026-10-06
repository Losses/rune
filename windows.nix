# Direct entry point for Windows native Nix / nova-nix
# Usage:
#   nova-nix build windows.nix
#   nova-nix eval windows.nix

import ./nix/platforms/windows/default.nix { }
