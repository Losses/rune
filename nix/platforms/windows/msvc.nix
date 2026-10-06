# Portable MSVC and Windows SDK toolchain for Windows-native Nix / nova-nix.
# Supports both x86_64 (x64) and Windows on ARM (arm64).
#
# Unlike the official Visual Studio Installer, this derivation avoids machine-level
# registry writes and administrative elevation, using portable file layouts (similar
# to portable-msvc / xwin).
#
# It provides:
# 1. msvcPackage: The store path containing cl.exe, link.exe, and Windows SDK.
# 2. env: The exact environment variables (INCLUDE, LIB, PATH, VCINSTALLDIR, WindowsSdkDir)
#    required by rustc, cc-rs, CMake, and Ninja for MSVC builds.

{ lib ? null
, fetchurl ? (import <nix/fetchurl.nix>)
, customMsvcRoot ? null
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
, hostArch ? "x64"
}:

let
  version = "17.10";
  sdkVersion = "10.0.22621.0";

  # Store package for portable MSVC (when built in pure Nix store)
  msvcPackage = derivation {
    name = "portable-msvc-${arch}-${version}";
    system = system;
    builder = "builtin:unpack";

    srcs = [
      (fetchurl {
        url = "https://github.com/Losses/rune-toolchain-cache/releases/download/v1.0.0/portable-msvc-${arch}-${version}-${sdkVersion}.tar.zst";
        sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
      })
    ];
  };

  # Active root: either a store path or a user-specified host path
  root = if customMsvcRoot != null then customMsvcRoot else "${msvcPackage}";

  vcToolsDir = "${root}/VC/Tools/MSVC/${version}";
  winSdkDir = "${root}/Windows Kits/10";

in rec {
  inherit version sdkVersion arch hostArch msvcPackage;

  # Directory paths: adjusted dynamically for target arch (x64 vs arm64)
  paths = {
    # Compiler binary directory (e.g. Hostx64/x64 or Hostx64/arm64)
    vcBin = "${vcToolsDir}/bin/Host${hostArch}/${arch}";
    vcInclude = "${vcToolsDir}/include";
    # Libraries targeting chosen arch (lib/x64 or lib/arm64)
    vcLib = "${vcToolsDir}/lib/${arch}";
    # SDK host executables (rc.exe, mt.exe run on host)
    sdkBin = "${winSdkDir}/bin/${sdkVersion}/${hostArch}";
    sdkInclude = [
      "${winSdkDir}/Include/${sdkVersion}/ucrt"
      "${winSdkDir}/Include/${sdkVersion}/um"
      "${winSdkDir}/Include/${sdkVersion}/shared"
      "${winSdkDir}/Include/${sdkVersion}/winrt"
    ];
    sdkLib = [
      "${winSdkDir}/Lib/${sdkVersion}/ucrt/${arch}"
      "${winSdkDir}/Lib/${sdkVersion}/um/${arch}"
    ];
  };

  # Computed environment variables matching vcvars (vcvars64 or vcvarsamd64_arm64)
  env = {
    VCINSTALLDIR = "${vcToolsDir}/";
    WindowsSdkDir = "${winSdkDir}/";
    WindowsSDKVersion = "${sdkVersion}\\";

    # Headers (architecture-independent)
    INCLUDE = builtins.concatStringsSep ";" ([ paths.vcInclude ] ++ paths.sdkInclude);

    # Linker Libraries (needed by rustc / cc-rs / link.exe for target arch)
    LIB = builtins.concatStringsSep ";" ([ paths.vcLib ] ++ paths.sdkLib);

    # Executables on PATH (cl.exe, link.exe, rc.exe, mt.exe)
    PATH = "${paths.vcBin};${paths.sdkBin}";

    # C / C++ tool flags
    CC = "cl.exe";
    CXX = "cl.exe";
    AR = "lib.exe";
  };

  # Shell hook snippet for Windows PowerShell
  setupHookPwsh = ''
    $env:VCINSTALLDIR = "${env.VCINSTALLDIR}"
    $env:WindowsSdkDir = "${env.WindowsSdkDir}"
    $env:WindowsSDKVersion = "${env.WindowsSDKVersion}"
    $env:INCLUDE = "${env.INCLUDE}" + ";" + $env:INCLUDE
    $env:LIB = "${env.LIB}" + ";" + $env:LIB
    $env:PATH = "${env.PATH};" + $env:PATH
  '';
}
