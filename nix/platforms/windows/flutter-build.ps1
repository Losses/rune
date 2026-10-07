$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory $env:out | Out-Null
& tar.exe -xf $env:src -C $env:out
if ($LASTEXITCODE) { throw 'Flutter archive extraction failed' }
Move-Item "$env:out\flutter" "$env:out\sdk"
foreach ($required in @('bin\cache\dart-sdk\bin\dart.exe', 'bin\cache\flutter_tools.snapshot')) {
    if (-not (Test-Path "$env:out\sdk\$required")) { throw "Pinned Flutter archive missing $required" }
}
New-Item -ItemType Directory "$env:out\flutter\bin" -Force | Out-Null
Copy-Item $env:launcher "$env:out\flutter\bin\flutter-launcher.ps1"
foreach ($command in @('flutter', 'dart')) {
    $mode = if ($command -eq 'dart') { '1' } else { '0' }
    @"
@echo off
setlocal
set "RUNE_DART_LAUNCH=$mode"
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "$env:out\flutter\bin\flutter-launcher.ps1" %*
exit /b %errorlevel%
"@ | Set-Content "$env:out\flutter\bin\$command.bat" -Encoding ASCII
}
