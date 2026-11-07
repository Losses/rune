# Build a VS-shaped portable toolchain without running any installers.
{ fetchurl ? (import <nix/fetchurl.nix>)
, system ? "x86_64-windows"
, manifest ? import ./msvc-packages.nix
}:
let
  fetchPayload = payload:
    let
      source = fetchurl {
        inherit (payload) url sha256;
        name = builtins.replaceStrings [ " " ] [ "-" ] payload.fileName;
      };
    in "${source}\t${payload.fileName}";

in derivation {
  name = "msvc-${manifest.vcVersion}-sdk-${manifest.sdkVersion}";
  inherit system;
  inherit (manifest) vcVersion sdkVersion;
  builder = "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe";
  args = [ "-NoProfile" "-ExecutionPolicy" "Bypass" "-File" ./msvc-build.ps1 ];
  # Plain tab-separated records survive nova-nix's Windows store-path rewriting.
  vsixPayloads = builtins.concatStringsSep "\n" (map fetchPayload manifest.vsix);
  sdkPayloads = builtins.concatStringsSep "\n" (map fetchPayload manifest.sdk);
  PATH = "C:\\Windows\\System32;C:\\Windows\\System32\\WindowsPowerShell\\v1.0";
}
