# Microsoft DirectX Shader Compiler (dxcompiler.dll + dxil.dll) for Windows on ARM.
# Sourced from the official microsoft/DirectXShaderCompiler GitHub release.
#
# wgpu's DX12 backend (Dx12Compiler::Auto) links DXC statically only on x64
# (mach-dxcompiler-rs is not built for aarch64), otherwise it loads
# dxcompiler.dll by name, so the DLL must sit next to rune.exe, and falls back
# to the very slow FXC when it is missing.  dxil.dll is shipped with it: DXC
# loads it to validate and sign the DXIL, and D3D12 rejects unsigned shaders.
# Only the arm64 package needs these; x64 has DXC linked into rune.exe.

{ fetchurl ? (import <nix/fetchurl.nix>)
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

let
  version = "1.9.2609";

  # v1.9.2609 dxc_2026_09_29.zip; the hash matches the digest GitHub shows
  # for the release asset.  Contains bin/{x64,arm64,x86}, inc, lib and the
  # LICENSE files (LLVM/NCSA, MIT, Microsoft) at the top level.
  dxcZip = fetchurl {
    url = "https://github.com/microsoft/DirectXShaderCompiler/releases/download/v${version}/dxc_2026_09_29.zip";
    sha256 = "ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1";
  };

  # Unpack the whole archive like the CMake package; the CI step copies
  # bin\arm64\{dxcompiler,dxil}.dll and the LICENSE files into the release folder.
  package = derivation {
    name = "dxc-${version}";
    system = system;
    builder = "cmd.exe";
    args = [ "/c" "mkdir %out% && tar.exe -xf %src% -C %out%" ];
    src = dxcZip;
    PATH = "C:\\Windows\\System32";
  };

in rec {
  inherit version arch package;
}
