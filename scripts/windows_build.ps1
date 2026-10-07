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
. "$ScriptDir\windows_monitor.ps1"
$Diagnostics = Join-Path $ProjectRoot "build\diagnostics\$Arch"
$Monitor = @{ DiagnosticDirectory = $Diagnostics }

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
    $CargoArguments = @("build", "--release", "-p", "hub", "-vv", "--timings")
    if ($Arch -eq "arm64") { $CargoArguments += @("--target", $RustTarget) }
    Invoke-MonitoredCommand @Monitor -Stage "3-cargo" -FilePath cargo -ArgumentList $CargoArguments -TimeoutSeconds 2700
    if (-not (Test-Path $HubDll)) {
        throw "Failed to produce hub.dll at $HubDll"
    }
    Write-Host "  -> Successfully produced hub.dll and import library." -ForegroundColor Green
} else {
    Write-Host "`n[3/5] Skipping Rust build as requested." -ForegroundColor DarkGray
}

# Step 4: Ensure Flutter dependencies & patch cargokit.cmake
Write-Host "`n[4/5] Preparing Flutter and Patching CargoKit..." -ForegroundColor Yellow

# Flutter's first run in CI can hang during toolchain init (analytics ping +
# SDK cache validation) BEFORE pub resolution even starts. Suppress analytics
# and warm up with `flutter --version` first so a broken SDK fails fast and
# the hang is isolated to a named sub-step.
$env:FLUTTER_SUPPRESS_ANALYTICS = "true"
$env:FLUTTER_DISABLE_ANALYTICS = "true"
$env:CI = "true"

Write-Host "  -> [4a] Warming up Flutter toolchain (flutter --version)..." -ForegroundColor Cyan
Write-FlutterPreflight -DiagnosticDirectory $Diagnostics
Invoke-MonitoredCommand @Monitor -Stage "4a-flutter-version" -FilePath flutter -ArgumentList @("--suppress-analytics", "--version") -TimeoutSeconds 600
Write-Host "  -> [4a] Flutter toolchain warm-up finished." -ForegroundColor Green

Write-Host "  -> [4b] flutter pub get (network; cold cache can take minutes)..." -ForegroundColor Cyan
Invoke-MonitoredCommand @Monitor -Stage "4b-pub-get" -FilePath flutter -ArgumentList @("--suppress-analytics", "pub", "get", "--verbose") -TimeoutSeconds 900
Write-Host "  -> [4b] flutter pub get finished." -ForegroundColor Green

# Generate Dart bindings from Rust structs (lib/bindings/bindings.dart).
# rinf's cargokit build_tool only runs `cargo build`, so the Dart codegen is a
# separate explicit step here (same `rinf gen` call used by the other workflows).
Write-Host "  -> [4c] rinf gen (local codegen; no cargo/network)..." -ForegroundColor Cyan
Invoke-MonitoredCommand @Monitor -Stage "4c-rinf-gen" -FilePath rinf -ArgumentList @("gen") -TimeoutSeconds 600
Write-Host "  -> [4c] rinf gen finished." -ForegroundColor Green

# Locate cargokit.cmake in ephemeral plugin symlinks
$CargokitCmake = Join-Path $ProjectRoot "windows\flutter\ephemeral\.plugin_symlinks\rinf\cargokit\cmake\cargokit.cmake"
if (Test-Path $CargokitCmake) {
    Write-Host "  -> Patching cargokit.cmake to direct-link hub.dll.lib..." -ForegroundColor Cyan
    $CmakeLibPath = ($HubLib -replace '\\', '/')
    $Content = Get-Content -Raw $CargokitCmake
    
    # Refresh upstream or previously patched paths on every architecture switch.
    $rules = @(
        @('add_custom_target\("\$\{target\}_cargokit" DEPENDS [^\r\n]*\)', 'add_custom_target("${target}_cargokit" DEPENDS "' + $CmakeLibPath + '")'),
        @('target_link_libraries\("\$\{target\}" PRIVATE [^\r\n]*\)', 'target_link_libraries("${target}" PRIVATE "' + $CmakeLibPath + '")'),
        @('set\("\$\{target\}_cargokit_lib" [^\r\n]* PARENT_SCOPE\)', 'set("${target}_cargokit_lib" "' + $CmakeLibPath + '" PARENT_SCOPE)')
    )
    foreach ($rule in $rules) {
        if ([regex]::Matches($Content, $rule[0]).Count -ne 1) { throw "Unsupported CargoKit CMake layout: $($rule[0])" }
        $replacement = $rule[1]
        $Content = [regex]::Replace($Content, $rule[0], [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $replacement })
    }
    Set-Content -Path $CargokitCmake -Value $Content -NoNewline
} else { throw "CargoKit CMake file missing: $CargokitCmake" }

# Step 5: Build a full native release. Flutter targets the host Dart ABI.
$OutputDir = Join-Path $ProjectRoot "build\windows\$Arch\runner\Release"
if (-not $SkipFlutterBuild) {
    Invoke-MonitoredCommand @Monitor -Stage "5-flutter-build" -FilePath flutter -ArgumentList @("build", "windows", "--release") -TimeoutSeconds 2700
    if (-not (Test-Path $OutputDir -PathType Container)) { throw "No $Arch Release directory" }
    Copy-Item -Path $HubDll -Destination $OutputDir -Force
}
function Assert-PeArchitecture([string]$Path) {
    if (-not (Test-Path $Path -PathType Leaf)) { throw "Missing release binary: $Path" }
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.ReadUInt16() -ne 0x5A4D) { throw "Not a PE binary: $Path" }
        $reader.BaseStream.Position = 0x3C
        $offset = $reader.ReadInt32()
        if ($offset -lt 0 -or $offset -gt ($reader.BaseStream.Length - 6)) { throw "Invalid PE header: $Path" }
        $reader.BaseStream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $Path" }
        $machine = $reader.ReadUInt16()
        $expected = if ($Arch -eq 'arm64') { 0xAA64 } else { 0x8664 }
        if ($machine -ne $expected) { throw "Wrong PE architecture: $Path (expected $Arch, machine $machine)" }
    } finally { $reader.Dispose() }
}
foreach ($binary in @('rune.exe', 'hub.dll', 'flutter_windows.dll')) {
    Assert-PeArchitecture (Join-Path $OutputDir $binary)
}
foreach ($asset in @('data\app.so', 'data\icudtl.dat')) {
    $path = Join-Path $OutputDir $asset
    if (-not (Test-Path $path -PathType Leaf) -or (Get-Item $path).Length -eq 0) { throw "Missing/empty release asset: $path" }
}
$assets = Join-Path $OutputDir 'data\flutter_assets'
if (-not (Test-Path $assets -PathType Container) -or -not (Get-ChildItem $assets -File -Recurse | Select-Object -First 1)) {
    throw "Missing/empty Flutter assets: $assets"
}
Write-Host "Complete $Arch application validated: $OutputDir" -ForegroundColor Green

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
        throw "ISCC.exe not found; installer creation is required."
    } else {
        $IssFile = Join-Path $ProjectRoot "rune.iss"
        Write-Host "  -> Running: $IsccPath /DAppArch=$Arch $IssFile" -ForegroundColor Cyan
        $InstallerPath = Join-Path $ProjectRoot "Output\Rune-$Arch-Setup.exe"
        # The absolute target is the explicitly named installer, never a directory.
        if (Test-Path $InstallerPath) { Remove-Item -LiteralPath $InstallerPath -Force }
        & "$IsccPath" "/DAppArch=$Arch" "$IssFile"
        if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed: exit $LASTEXITCODE" }
        if (-not (Test-Path $InstallerPath -PathType Leaf) -or (Get-Item $InstallerPath).Length -eq 0) { throw "Installer output missing: $InstallerPath" }
        if (Test-Path $InstallerPath) {
            Write-Host "  -> Installer generated successfully at: $InstallerPath" -ForegroundColor Green
        }
    }
}

