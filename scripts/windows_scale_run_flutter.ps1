# Run from an interactive task against a dedicated validation checkout.
param(
  [Parameter(Mandatory)][string]$Checkout,
  [Parameter(Mandatory)][string]$FlutterBin,
  [Parameter(Mandatory)][string]$LogPath,
  [switch]$VisualizerReadback
)
$ErrorActionPreference = 'Stop'
try {
  if (Get-Process rune -ErrorAction SilentlyContinue) { throw 'An existing Rune process is running; preserve the user session.' }
  if (-not (Test-Path (Join-Path $Checkout 'pubspec.yaml'))) { throw 'Validation checkout not found.' }
  if (-not (Test-Path (Join-Path $FlutterBin 'flutter.bat'))) { throw 'Flutter SDK not found.' }
  Set-Location -LiteralPath $Checkout
  $env:PATH = "$FlutterBin;$env:PATH"
  if ($VisualizerReadback) { $env:RUNE_VISUALIZER_READBACK = '1' }
  # The extracted isolated SDK can be owned by Administrators. Permit only
  # this SDK in child Git processes; do not alter the user's Git config.
  # Resolve-Path preserves NTFS 8.3 aliases, whereas Git canonicalizes them.
  Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class RuneSdkPath {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern uint GetLongPathName(string path, StringBuilder result, uint size);
}
'@
  $longPath = New-Object System.Text.StringBuilder 32768
  $pathLength = [RuneSdkPath]::GetLongPathName((Resolve-Path -LiteralPath $FlutterBin).Path, $longPath, 32768)
  if ($pathLength -eq 0 -or $pathLength -ge 32768) { throw 'Could not canonicalize Flutter SDK path.' }
  $sdkRoot = Split-Path -Parent $longPath.ToString()
  $configIndex = 0
  if ($env:GIT_CONFIG_COUNT) { $configIndex = [int]$env:GIT_CONFIG_COUNT }
  [Environment]::SetEnvironmentVariable("GIT_CONFIG_KEY_$configIndex", 'safe.directory', 'Process')
  [Environment]::SetEnvironmentVariable("GIT_CONFIG_VALUE_$configIndex", $sdkRoot.Replace('\', '/'), 'Process')
  $env:GIT_CONFIG_COUNT = [string]($configIndex + 1)
  . (Join-Path $Checkout 'scripts/windows_setup_msvc_env.ps1') -Arch x64
  $vsTools = Join-Path ${env:ProgramFiles} 'Microsoft Visual Studio\2022\Community\Common7\IDE\CommonExtensions\Microsoft\CMake'
  $env:PATH = "$(Join-Path $vsTools 'CMake\bin');$(Join-Path $vsTools 'Ninja');$env:PATH"
  # Windows PowerShell 5 treats native stderr as an error under Stop. Flutter
  # writes normal diagnostics there; judge the native exit code instead.
  $ErrorActionPreference = 'Continue'
  # package_config.json retains absolute SDK package paths. Refresh it when
  # switching SDKs, or --no-pub can silently compile the previous framework.
  & (Join-Path $FlutterBin 'flutter.bat') pub get --offline --enforce-lockfile *> $LogPath
  if ($LASTEXITCODE -ne 0) { throw 'Locked offline dependency resolution failed.' }
  . (Join-Path $Checkout 'scripts/windows_verify_flutter_framework.ps1')
  Assert-RuneFlutterFramework -Checkout $Checkout
  & (Join-Path $FlutterBin 'flutter.bat') run -d windows --debug --no-pub --dart-entrypoint-args=--profile --dart-entrypoint-args=scale-validation *>> $LogPath
  exit $LASTEXITCODE
} catch {
  $_ | Out-String | Add-Content -Encoding UTF8 -LiteralPath $LogPath
  exit 1
}
