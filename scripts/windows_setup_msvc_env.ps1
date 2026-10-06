<#
.SYNOPSIS
  Sets up the MSVC + Windows SDK environment for Rune without requiring Visual Studio IDE
  or admin elevation, matching the Nix Windows platform derivation configuration.
  Supports both x64 and arm64 targets.
#>

param(
    [ValidateSet("x64", "arm64")]
    [string]$Arch = "x64",
    [string]$MsvcPath = $env:VCINSTALLDIR
)

Write-Host "Configuring Rune MSVC Environment (Target: $Arch)..." -ForegroundColor Cyan

# Check if cl.exe is already on PATH for the desired arch
if (Get-Command cl.exe -ErrorAction SilentlyContinue) {
    if ($env:VSCMD_ARG_TGT_ARCH -eq $Arch -or (-not $env:VSCMD_ARG_TGT_ARCH -and $Arch -eq "x64")) {
        Write-Host "[OK] cl.exe already on PATH for $Arch." -ForegroundColor Green
        return
    }
}

# If not on PATH or target arch differs, attempt to locate Visual Studio or Build Tools
if (-not $MsvcPath) {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsInstallPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($vsInstallPath) {
            $vcvarsFile = if ($Arch -eq "arm64") { "vcvarsamd64_arm64.bat" } else { "vcvars64.bat" }
            $vcvars = Join-Path $vsInstallPath "VC\Auxiliary\Build\$vcvarsFile"
            if (Test-Path $vcvars) {
                Write-Host "Found MSVC via vswhere: $vcvars ($Arch)" -ForegroundColor Yellow
                # Extract environment from vcvars
                cmd /c "`"$vcvars`" > nul && set" | ForEach-Object {
                    if ($_ -match "^(.*?)=(.*)$") {
                        Set-Item -Path "env:\$($matches[1])" -Value $matches[2]
                    }
                }
                Write-Host "[OK] MSVC ($Arch) environment imported successfully." -ForegroundColor Green
                return
            }
        }
    }
}

Write-Host "Warning: Could not auto-detect MSVC for $Arch. Please set VCINSTALLDIR or run from Native Tools Command Prompt." -ForegroundColor Yellow
