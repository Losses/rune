# Configure the target MSVC environment, preferring native host tools.
param(
    [ValidateSet("x64", "arm64")]
    [string]$Arch = "x64",
    [string]$MsvcPath = $env:VCINSTALLDIR
)
$ErrorActionPreference = 'Stop'
$nativeArch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
$hostArch = if ($nativeArch -eq 'ARM64') { 'arm64' } else { 'amd64' }
if ((Get-Command cl.exe -ErrorAction SilentlyContinue) -and $env:VSCMD_ARG_TGT_ARCH -eq $Arch) {
    Write-Host "MSVC already configured for $Arch"
    return
}
if (-not $MsvcPath) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path $vswhere)) { throw 'vswhere.exe not found; install Visual Studio C++ tools' }
    $component = if ($Arch -eq 'arm64') { 'Microsoft.VisualStudio.Component.VC.Tools.ARM64' } else { 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' }
    $installation = & $vswhere -latest -products * -requires $component -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $installation) { throw "Visual Studio tools not found for $Arch" }
    $MsvcPath = Join-Path $installation 'VC'
}
$vcvars = Join-Path $MsvcPath 'Auxiliary\Build\vcvarsall.bat'
if (-not (Test-Path $vcvars)) { throw "Missing vcvarsall.bat: $vcvars" }
$targetArch = if ($Arch -eq 'x64') { 'amd64' } else { 'arm64' }
$vcArgument = if ($hostArch -eq $targetArch) { $hostArch } else { "${hostArch}_$targetArch" }
$environment = & cmd.exe /d /c "`"$vcvars`" $vcArgument >nul && set"
if ($LASTEXITCODE -ne 0) { throw "vcvarsall failed for $vcArgument (exit $LASTEXITCODE)" }
foreach ($line in $environment) {
    if ($line -match '^([^=]+)=(.*)$') { Set-Item -Path "env:\$($matches[1])" -Value $matches[2] }
}
if (-not (Get-Command cl.exe -ErrorAction SilentlyContinue) -or $env:VSCMD_ARG_TGT_ARCH -ne $Arch) {
    throw "MSVC did not configure the requested $Arch target"
}
Write-Host "MSVC configured: host=$hostArch target=$Arch" -ForegroundColor Green
