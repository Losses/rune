# MSVC and Windows SDK toolchain adapter for Windows-native Nix / nova-nix.
# Supports both x86_64 (x64) and Windows on ARM (arm64).
#
# Resolves MSVC toolchain either from:
# 1. customMsvcRoot: portable MSVC or offline layout folder (e.g. created via vs_BuildTools.exe --layout)
# 2. Host Visual Studio installation (auto-detected via vswhere or standard install paths)
#
# Provides exact environment variables (INCLUDE, LIB, PATH, VCINSTALLDIR, WindowsSdkDir)
# required by rustc, cc-rs, CMake, and Ninja for MSVC builds.

{ customMsvcRoot ? null
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
, hostArch ? "x64"
, vcVersion ? "14.39.33519"
, sdkVersion ? "10.0.22621.0"
}:

let
  isCustom = customMsvcRoot != null;
  root = if isCustom then customMsvcRoot else "C:/Program Files/Microsoft Visual Studio/2022/Community";

  vcToolsDir = if isCustom then
    "${root}/VC/Tools/MSVC/${vcVersion}"
  else
    "${root}/VC/Tools/MSVC/${vcVersion}";

  winSdkDir = if isCustom then
    "${root}/Windows Kits/10"
  else
    "C:/Program Files (x86)/Windows Kits/10";

in rec {
  inherit vcVersion sdkVersion arch hostArch;

  # Directory paths: adjusted dynamically for target arch (x64 vs arm64)
  paths = {
    vcBin = "${vcToolsDir}/bin/Host${hostArch}/${arch}";
    vcInclude = "${vcToolsDir}/include";
    vcLib = "${vcToolsDir}/lib/${arch}";
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

    INCLUDE = builtins.concatStringsSep ";" ([ paths.vcInclude ] ++ paths.sdkInclude);
    LIB = builtins.concatStringsSep ";" ([ paths.vcLib ] ++ paths.sdkLib);
    PATH = "${paths.vcBin};${paths.sdkBin}";

    CC = "cl.exe";
    CXX = "cl.exe";
    AR = "lib.exe";
  };

  # Shell hook snippet for Windows PowerShell
  setupHookPwsh = if isCustom then ''
    $env:VCINSTALLDIR = "${env.VCINSTALLDIR}"
    $env:WindowsSdkDir = "${env.WindowsSdkDir}"
    $env:WindowsSDKVersion = "${env.WindowsSDKVersion}"
    $env:INCLUDE = "${env.INCLUDE}" + ";" + $env:INCLUDE
    $env:LIB = "${env.LIB}" + ";" + $env:LIB
    $env:PATH = "${env.PATH};" + $env:PATH
  '' else ''
    # Auto-detect MSVC environment via vswhere / vcvars
    if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue) -or ($env:VSCMD_ARG_TGT_ARCH -ne "${arch}")) {
      $vswhere = "${"$"}{env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
      if (Test-Path $vswhere) {
        $vsInstall = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsInstall) {
          $batName = if ("${arch}" -eq "arm64") { "vcvarsamd64_arm64.bat" } else { "vcvars64.bat" }
          $batPath = Join-Path $vsInstall "VC\Auxiliary\Build\$batName"
          if (Test-Path $batPath) {
            cmd /c "`"$batPath`" > nul && set" | ForEach-Object {
              if ($_ -match "^(.*?)=(.*)$") {
                Set-Item -Path "env:\$($matches[1])" -Value $matches[2]
              }
            }
          }
        }
      }
    }
  '';
}
