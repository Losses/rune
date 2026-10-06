<#
.SYNOPSIS
  Sets up the MSVC + Windows SDK environment for Rune without requiring Visual Studio IDE
  or admin elevation, matching the Nix Windows platform derivation configuration.
#>

param(
    [string]$MsvcPath = $env:VCINSTALLDIR
)

Write-Host "Configuring Rune MSVC Environment..." -ForegroundColor Cyan

# Check if cl.exe is already on PATH
if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    Write-Host "[OK] cl.exe already found on PATH." -ForegroundColor Green
    return
}

# If not on PATH, attempt to locate Visual Studio or Build Tools
if (-not $MsvcPath) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsInstallPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsInstallPath) {
            $vcvars = Join-Path $vsInstallPath "VC\Auxiliary\Build\vcvars64.bat"
            if (Test-Path $vcvars) {
                Write-Host "Found MSVC via vswhere: $vcvars" -ForegroundColor Yellow
                # Extract environment from vcvars64
                cmd /c "`"$vcvars`" > nul && set" | ForEach-Object {
                    if ($_ -match "^(.*?)=(.*)$") {
                        Set-Item -Path "env:\$($matches[1])" -Value $matches[2]
                    }
                }
                Write-Host "[OK] MSVC environment imported successfully." -ForegroundColor Green
                return
            }
        }
    }
}

Write-Host "Warning: Could not auto-detect MSVC. Please set VCINSTALLDIR or run from Native Tools Command Prompt." -ForegroundColor Yellow
