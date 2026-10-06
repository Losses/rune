# Portable MSVC and Windows SDK toolchain for Windows-native Nix / nova-nix.
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
}:

let
  # Default release of pre-extracted portable-msvc bundle (VCTools + WinSDK)
  # Can be overridden or substituted via binary cache / nova-cache.
  version = "17.10";
  sdkVersion = "10.0.22621.0";

  # Store package for portable MSVC (when built in pure Nix store)
  msvcPackage = derivation {
    name = "portable-msvc-${version}";
    system = "x86_64-windows";
    builder = "builtin:unpack";

    # The archive should contain:
    #   VC/Tools/MSVC/<version>/bin/Hostx64/x64/...
    #   VC/Tools/MSVC/<version>/include/...
    #   VC/Tools/MSVC/<version>/lib/x64/...
    #   Windows Kits/10/Include/<sdkVersion>/...
    #   Windows Kits/10/Lib/<sdkVersion>/...
    #   Windows Kits/10/bin/<sdkVersion>/x64/...
    srcs = [
      (fetchurl {
        url = "https://github.com/Losses/rune-toolchain-cache/releases/download/v1.0.0/portable-msvc-${version}-${sdkVersion}.tar.zst";
        sha256 = "0000000000000000000000000000000000000000000000000000000000000000"; # Pin hash when uploaded
      })
    ];
  };

  # Active root: either a store path or a user-specified host path
  root = if customMsvcRoot != null then customMsvcRoot else "${msvcPackage}";

  vcToolsDir = "${root}/VC/Tools/MSVC/${version}";
  winSdkDir = "${root}/Windows Kits/10";

in rec {
  inherit version sdkVersion msvcPackage;

  # Directory paths
  paths = {
    vcBin = "${vcToolsDir}/bin/Hostx64/x64";
    vcInclude = "${vcToolsDir}/include";
    vcLib = "${vcToolsDir}/lib/x64";
    sdkBin = "${winSdkDir}/bin/${sdkVersion}/x64";
    sdkInclude = [
      "${winSdkDir}/Include/${sdkVersion}/ucrt"
      "${winSdkDir}/Include/${sdkVersion}/um"
      "${winSdkDir}/Include/${sdkVersion}/shared"
      "${winSdkDir}/Include/${sdkVersion}/winrt"
    ];
    sdkLib = [
      "${winSdkDir}/Lib/${sdkVersion}/ucrt/x64"
      "${winSdkDir}/Lib/${sdkVersion}/um/x64"
    ];
  };

  # Computed environment variables matching vcvars64.bat
  env = {
    VCINSTALLDIR = "${vcToolsDir}/";
    WindowsSdkDir = "${winSdkDir}/";
    WindowsSDKVersion = "${sdkVersion}\\";

    # Headers
    INCLUDE = builtins.concatStringsSep ";" ([ paths.vcInclude ] ++ paths.sdkInclude);

    # Linker Libraries (needed by rustc / cc-rs / link.exe)
    LIB = builtins.concatStringsSep ";" ([ paths.vcLib ] ++ paths.sdkLib);

    # Executables on PATH (cl.exe, link.exe, rc.exe, mt.exe)
    PATH = "${paths.vcBin};${paths.sdkBin}";

    # C / C++ tool flags
    CC = "cl.exe";
    CXX = "cl.exe";
    AR = "lib.exe";
  };

  # Shell hook snippet for Windows PowerShell or CMD
  setupHookPwsh = ''
    $env:VCINSTALLDIR = "${env.VCINSTALLDIR}"
    $env:WindowsSdkDir = "${env.WindowsSdkDir}"
    $env:WindowsSDKVersion = "${env.WindowsSDKVersion}"
    $env:INCLUDE = "${env.INCLUDE}" + ";" + $env:INCLUDE
    $env:LIB = "${env.LIB}" + ";" + $env:LIB
    $env:PATH = "${env.PATH};" + $env:PATH
  '';
}
