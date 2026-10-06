# Development Shell for Rune on Windows (MSVC)
#
# Combines:
# - Portable MSVC + Windows SDK (cl.exe, link.exe, Windows headers & libs)
# - Rust toolchain targeting x86_64-pc-windows-msvc
# - Flutter SDK for Windows
# - CMake, Ninja, rinf_cli

{ msvc
, rust
, flutter
, tools
}:

derivation {
  name = "rune-windows-devshell";
  system = "x86_64-windows";
  builder = "builtin:unpack"; # Dummy builder for evaluation / shell realization
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
  CARGO_BUILD_TARGET = rust.env.CARGO_BUILD_TARGET;

  # Combined search PATH
  PATH = builtins.concatStringsSep ";" [
    msvc.env.PATH
    rust.binPath
    flutter.binPath
    (builtins.concatStringsSep ";" tools.binPaths)
  ];

  # Combined PowerShell activation snippet
  shellHookPwsh = ''
    Write-Host "==========================================================" -ForegroundColor Cyan
    Write-Host "  Rune Windows Development Environment (MSVC)" -ForegroundColor Green
    Write-Host "  Toolchains: MSVC ${msvc.version}, Rust ${rust.version}, Flutter ${flutter.version}" -ForegroundColor Gray
    Write-Host "==========================================================" -ForegroundColor Cyan
    ${msvc.setupHookPwsh}
    ${rust.setupHookPwsh}
    ${flutter.setupHookPwsh}
    ${tools.setupHookPwsh}
  '';
}
