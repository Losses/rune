<#
.SYNOPSIS
  One-click automated build script for Rune on Windows using MSVC.
  Supports both x64 and arm64 (Windows on ARM) targets.
  
  Features:
  1. Configures MSVC + WinSDK environment for x64 or arm64.
  2. Injects rustup.cmd shim to intercept CargoKit and enforce Nix/MSVC toolchains.
  3. Prebuilds Rust core (hub.dll & hub.dll.lib) for the chosen target.
  4. Intercepts CMake cargokit linking via rinf.patch pattern.
  5. Builds the Flutter Windows release runner.
  6. Bundles complete runnable package into build/windows/<arch>/runner/Release.
#>

param(
    [ValidateSet("x64", "arm64")]
    [string]$Arch = "x64",
    [switch]$SkipRustBuild = $false,
    [switch]$SkipFlutterBuild = $false,
    [switch]$BuildInstaller = $false
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$ProjectRoot = Resolve-Path "$ScriptDir\.."

Set-Location $ProjectRoot
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  Rune Windows One-Click MSVC Build (Target: $Arch)" -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Cyan

# Step 1: Set up MSVC environment for target architecture
Write-Host "`n[1/5] Setting up MSVC & Windows SDK Environment ($Arch)..." -ForegroundColor Yellow
. "$ScriptDir\windows_setup_msvc_env.ps1" -Arch $Arch

# Step 2: Inject Shims into PATH (CargoKit interception)
$RustTarget = if ($Arch -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
Write-Host "`n[2/5] Injecting CargoKit Interception Shims (Target: $RustTarget)..." -ForegroundColor Yellow
$ShimDir = Join-Path $ProjectRoot "nix\platforms\windows\shims"
$env:PATH = "$ShimDir;$env:PATH"
# Cargo treats CARGO_BUILD_TARGET exactly like --target, so artifacts for BOTH
# architectures land under target/<triple>/release (never under plain target/release).
$env:CARGO_BUILD_TARGET = $RustTarget
# fdk-aac-sys 0.5.0 is vendored at native/fdk-aac-sys with an MSVC ARM64 patch
# in FDK_archdef.h (see [patch.crates-io] in Cargo.toml). No CFLAGS override needed.

# Step 3: Build Rust Hub (hub.dll + hub.dll.lib)
# CARGO_BUILD_TARGET is set for both arches, so the target subdirectory is always used.
$TargetDir = "target\$RustTarget\release"
$HubDll = Join-Path $ProjectRoot "$TargetDir\hub.dll"
$HubLib = Join-Path $ProjectRoot "$TargetDir\hub.dll.lib"

if (-not $SkipRustBuild) {
    Write-Host "`n[3/5] Compiling Rust Core (hub.dll for $RustTarget)..." -ForegroundColor Yellow
    if ($Arch -eq "arm64") {
        cargo build --release --target $RustTarget -p hub
    } else {
        cargo build --release -p hub
    }
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE (see compiler output above)"
    }
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
if ($LASTEXITCODE -ne 0) {
    throw "flutter pub get failed with exit code $LASTEXITCODE"
}

# Generate Dart bindings from Rust structs (lib/bindings/bindings.dart).
# rinf's cargokit build_tool only runs `cargo build`, so the Dart codegen is a
# separate explicit step here (same `rinf gen` call used by the other workflows).
rinf gen
if ($LASTEXITCODE -ne 0) {
    throw "rinf gen failed with exit code $LASTEXITCODE"
}

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
$OutputDir = Join-Path $ProjectRoot "build\windows\$Arch\runner\Release"
if (-not $SkipFlutterBuild) {
    Write-Host "`n[5/5] Building Flutter Windows Application (MSVC - $Arch)..." -ForegroundColor Yellow
    if ($Arch -eq "arm64") {
        flutter build windows --release --arm64
    } else {
        flutter build windows --release
    }
    if ($LASTEXITCODE -ne 0) {
        throw "flutter build windows failed with exit code $LASTEXITCODE"
    }
    
    # Ensure hub.dll is placed in release directory alongside rune.exe
    Copy-Item -Path $HubDll -Destination $OutputDir -Force
    Write-Host "`n==========================================================" -ForegroundColor Green
    Write-Host "  Build Complete ($Arch)! Standalone application located at:" -ForegroundColor Green
    Write-Host "  $OutputDir" -ForegroundColor Cyan
    Write-Host "==========================================================" -ForegroundColor Green
} else {
    Write-Host "`n[5/5] Skipping Flutter build as requested." -ForegroundColor DarkGray
}

# Step 6 (Optional): Package installer via Inno Setup
if ($BuildInstaller) {
    Write-Host "`n[6/6] Packaging Installer with Inno Setup (ISCC)..." -ForegroundColor Yellow
    $IsccCmd = Get-Command iscc.exe -ErrorAction SilentlyContinue
    $IsccPath = if ($IsccCmd) { $IsccCmd.Source } else {
        $DefaultPaths = @(
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles}\Inno Setup 6\ISCC.exe"
        )
        $DefaultPaths | Where-Object { Test-Path $_ } | Select-Object -First 1
    }
    
    if (-not $IsccPath) {
        Write-Warning "ISCC.exe (Inno Setup) not found. Skipping installer creation. (Install with: choco install innosetup -y)"
    } else {
        $IssFile = Join-Path $ProjectRoot "rune.iss"
        Write-Host "  -> Running: $IsccPath /DAppArch=$Arch $IssFile" -ForegroundColor Cyan
        & "$IsccPath" "/DAppArch=$Arch" "$IssFile"
        $InstallerPath = Join-Path $ProjectRoot "Output\Rune-$Arch-Setup.exe"
        if (Test-Path $InstallerPath) {
            Write-Host "  -> Installer generated successfully at: $InstallerPath" -ForegroundColor Green
        }
    }
}

