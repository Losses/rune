@echo off
rem ==============================================================================
rem rustup shim for CargoKit on Windows
rem Supports both x86_64 and aarch64 MSVC targets via %CARGO_BUILD_TARGET%
rem ==============================================================================

set TARGET=%CARGO_BUILD_TARGET%
if "%TARGET%"=="" set TARGET=x86_64-pc-windows-msvc

if "%1"=="toolchain" (
    if "%2"=="list" (
        echo stable-%TARGET% (default)
        exit /b 0
    )
    if "%2"=="install" (
        echo rustup-shim: toolchain is managed by Nix/MSVC, skipping install of '%3'
        exit /b 0
    )
    echo rustup-shim: unsupported 'toolchain %2' >&2
    exit /b 1
)

if "%1"=="target" (
    if "%2"=="list" (
        echo %TARGET%
        exit /b 0
    )
    if "%2"=="add" (
        echo rustup-shim: target managed by Nix/MSVC, skipping add of '%4'
        exit /b 0
    )
    echo rustup-shim: unsupported 'target %2' >&2
    exit /b 1
)

if "%1"=="component" (
    if "%2"=="add" exit /b 0
    echo rustup-shim: unsupported 'component %2' >&2
    exit /b 1
)

if "%1"=="run" (
    shift
    shift
    %1 %2 %3 %4 %5 %6 %7 %8 %9
    exit /b %ERRORLEVEL%
)

if "%1"=="--version" (
    echo rustup 1.28.2 (nix-windows-shim, target %TARGET%)
    exit /b 0
)

rem Fallback: forward to cargo directly
cargo %*
exit /b %ERRORLEVEL%
