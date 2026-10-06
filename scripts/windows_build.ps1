<#
.SYNOPSIS
  One-click automated build script for Rune on Windows using MSVC.
  
  Features:
  1. Configures MSVC + WinSDK environment without admin rights.
  2. Injects rustup.cmd shim to intercept CargoKit and enforce Nix/MSVC toolchains.
  3. Prebuilds Rust core (hub.dll & hub.dll.lib).
  4. Intercepts CMake cargokit linking via rinf.patch pattern.
  5. Builds the Flutter Windows release runner.
  6. Bundles complete runnable package into build/windows/x64/runner/Release.
#>

param(
    [switch]$SkipRustBuild = $false,
    [switch]$SkipFlutterBuild = $false
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ProjectRoot = Resolve-Path "$ScriptDir\.."

Set-Location $ProjectRoot
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Rune Windows One-Click MSVC Build" -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Cyan

# Step 1: Set up MSVC environment
Write-Host "`n[1/5] Setting up MSVC & Windows SDK Environment..." -ForegroundColor Yellow
. "$ScriptDir\windows_setup_msvc_env.ps1"

# Step 2: Inject Shims into PATH (CargoKit interception)
Write-Host "`n[2/5] Injecting CargoKit Interception Shims..." -ForegroundColor Yellow
$ShimDir = Join-Path $ProjectRoot "nix\platforms\windows\shims"
$env:PATH = "$ShimDir;$env:PATH"
$env:CARGO_BUILD_TARGET = "x86_64-pc-windows-msvc"

# Step 3: Build Rust Hub (hub.dll + hub.dll.lib)
$HubDll = Join-Path $ProjectRoot "target\release\hub.dll"
$HubLib = Join-Path $ProjectRoot "target\release\hub.dll.lib"

if (-not $SkipRustBuild) {
    Write-Host "`n[3/5] Compiling Rust Core (hub.dll)..." -ForegroundColor Yellow
    cargo build --release -p hub
    if (-not (Test-Path $HubDll)) {
        throw "Failed to produce hub.dll at $HubDll"
    }
    Write-Host "  -> Successfully produced hub.dll and import library." -ForegroundColor Green
} else {
    Write-Host "`n[3/5] Skipping Rust build as requested." -ForegroundColor DarkGray
}

# Step 4: Ensure Flutter dependencies & patch cargokit.cmake
Write-Host "`n[4/5] Preparing Flutter and Patching CargoKit..." -ForegroundColor Yellow
flutter pub get

# Locate cargokit.cmake in ephemeral plugin symlinks
$CargokitCmake = Join-Path $ProjectRoot "windows\flutter\ephemeral\.plugin_symlinks\rinf\cargokit\cmake\cargokit.cmake"
if (Test-Path $CargokitCmake) {
    Write-Host "  -> Patching cargokit.cmake to direct-link hub.dll.lib..." -ForegroundColor Cyan
    $CmakeLibPath = ($HubLib -replace '\\', '/')
    $Content = Get-Content -Raw $CargokitCmake
    
    # Replace CMake target dependencies and link targets if not already patched
    if ($Content -match 'add_custom_target\("\$\{target\}_cargokit" DEPENDS \$\{OUTPUT_LIB\}\)') {
        $Content = $Content -replace 'add_custom_target\("\$\{target\}_cargokit" DEPENDS \$\{OUTPUT_LIB\}\)', "add_custom_target(`"`${target}_cargokit`" DEPENDS `"$CmakeLibPath`")"
        $Content = $Content -replace 'target_link_libraries\("\$\{target\}" PRIVATE "\$\{OUTPUT_LIB\}\$\{IMPORT_LIB_EXTENSION\}"\)', "target_link_libraries(`"`${target}`" PRIVATE `"$CmakeLibPath`")"
        $Content = $Content -replace 'set\("\$\{target\}_cargokit_lib" \$\{OUTPUT_LIB\} PARENT_SCOPE\)', "set(`"`${target}_cargokit_lib`" `"$CmakeLibPath`" PARENT_SCOPE)"
        Set-Content -Path $CargokitCmake -Value $Content -NoNewline
        Write-Host "  -> cargokit.cmake successfully intercepted." -ForegroundColor Green
    } else {
        Write-Host "  -> cargokit.cmake already patched or modified." -ForegroundColor Gray
    }
}

# Step 5: Build Flutter Windows Release
$OutputDir = Join-Path $ProjectRoot "build\windows\x64\runner\Release"
if (-not $SkipFlutterBuild) {
    Write-Host "`n[5/5] Building Flutter Windows Application (MSVC)..." -ForegroundColor Yellow
    flutter build windows --release
    
    # Ensure hub.dll is placed in release directory alongside rune.exe
    Copy-Item -Path $HubDll -Destination $OutputDir -Force
    Write-Host "`n==========================================================" -ForegroundColor Green
    Write-Host "  Build Complete! Standalone application located at:" -ForegroundColor Green
    Write-Host "  $OutputDir" -ForegroundColor Cyan
    Write-Host "==========================================================" -ForegroundColor Green
} else {
    Write-Host "`n[5/5] Skipping Flutter build as requested." -ForegroundColor DarkGray
}
