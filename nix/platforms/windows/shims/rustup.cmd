@echo off
rem ==============================================================================
rem rustup shim for CargoKit on Windows
rem
rem CargoKit (rinf build layer) hardcodes checks for rustup and runs:
rem   rustup run stable cargo build ...
rem This shim intercepts CargoKit commands and delegates directly to the
rem Nix/MSVC toolchain, preventing Cargokit from pulling down external toolchains.
rem ==============================================================================

if "%1"=="toolchain" (
    if "%2"=="list" (
        echo stable-x86_64-pc-windows-msvc (default)
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
        echo x86_64-pc-windows-msvc
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
    rem Shift past 'run' and toolchain name (e.g., 'stable')
    shift
    shift
    %1 %2 %3 %4 %5 %6 %7 %8 %9
    exit /b %ERRORLEVEL%
)

if "%1"=="--version" (
    echo rustup 1.28.2 (nix-windows-shim)
    exit /b 0
)

rem Fallback: forward to cargo directly
cargo %*
exit /b %ERRORLEVEL%
