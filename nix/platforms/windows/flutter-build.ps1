$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory $env:out | Out-Null
# The wrapper output contains no SDK tree or junction for nova-nix to copy.
$source = $env:sdk.Replace('/', '\')
Set-Content "$env:out\sdk-path.txt" $source -Encoding UTF8
foreach ($required in @('bin\cache\dart-sdk\bin\dart.exe', 'bin\cache\flutter_tools.snapshot')) {
    if (-not (Test-Path "$source\$required")) { throw "Pinned Flutter archive missing $required" }
}
$dartSdk = $env:dartSdk.Replace('/', '\')
if (-not (Test-Path "$dartSdk\bin\dart.exe")) { throw "Pinned native Dart missing: $dartSdk" }
Set-Content "$env:out\dart-sdk-path.txt" $dartSdk -Encoding UTF8
Set-Content "$env:out\architecture.txt" $env:arch -Encoding ASCII
New-Item -ItemType Directory "$env:out\flutter\bin" -Force | Out-Null
Copy-Item $env:launcher "$env:out\flutter\bin\flutter-launcher.ps1"
foreach ($command in @('flutter', 'dart')) {
    $mode = if ($command -eq 'dart') { '1' } else { '0' }
    @"
@echo off
setlocal
set "RUNE_DART_LAUNCH=$mode"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0flutter-launcher.ps1" %*
exit /b %errorlevel%
"@ | Set-Content "$env:out\flutter\bin\$command.bat" -Encoding ASCII
}
