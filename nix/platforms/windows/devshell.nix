# Development Shell for Rune on Windows (MSVC)
# Supports both x86_64 (x64) and Windows on ARM (arm64).
#
# Combines:
# - Portable MSVC + Windows SDK (cl.exe, link.exe, Windows headers & libs for target arch)
# - Rust toolchain targeting MSVC (x86_64-pc-windows-msvc or aarch64-pc-windows-msvc)
# - Flutter SDK for Windows
# - CMake, Ninja, rinf_cli

{ msvc
, rust
, flutter
, tools
, system ? "x86_64-windows"
, arch ? (if system == "aarch64-windows" then "arm64" else "x64")
}:

derivation {
  name = "rune-windows-devshell-${arch}";
  system = system;
  builder = "builtin:unpack";
  srcs = [];

  # Environment variables for MSVC, Rust, and Flutter
  VCINSTALLDIR = msvc.env.VCINSTALLDIR;
  WindowsSdkDir = msvc.env.WindowsSdkDir;
  WindowsSDKVersion = msvc.env.WindowsSDKVersion;
  INCLUDE = msvc.env.INCLUDE;
  LIB = msvc.env.LIB;
  CC = msvc.env.CC;
  CXX = msvc.env.CXX;
  AR = msvc.env.AR;

  RUST_BACKTRACE = rust.env.RUST_BACKTRACE;
  CARGO_BUILD_TARGET = rust.target;

  # Combined search PATH (shims placed first to intercept CargoKit)
  PATH = builtins.concatStringsSep ";" [
    "${./shims}"
    msvc.env.PATH
    rust.binPath
    flutter.binPath
    (builtins.concatStringsSep ";" tools.binPaths)
  ];

  # Combined PowerShell activation snippet
  shellHookPwsh = ''
    Write-Host "==========================================================" -ForegroundColor Cyan
    Write-Host "  Rune Windows Development Environment (MSVC - ${arch})" -ForegroundColor Green
    Write-Host "  Toolchains: MSVC ${msvc.version}, Rust ${rust.version} (${rust.target})" -ForegroundColor Gray
    Write-Host "==========================================================" -ForegroundColor Cyan
    ${msvc.setupHookPwsh}
    ${rust.setupHookPwsh}
    ${flutter.setupHookPwsh}
    ${tools.setupHookPwsh}
  '';
}
