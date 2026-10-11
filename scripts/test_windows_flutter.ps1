param([Parameter(Mandatory)][string]$Package)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\windows_monitor.ps1"
$diagnostics = 'build/diagnostics/flutter-smoke'
$source = (Get-Content (Join-Path $Package 'sdk-path.txt') -Raw).Trim()
$root = Join-Path $env:LOCALAPPDATA ('Rune\Flutter\' + (Split-Path $Package -Leaf))
# Metadata inventory detects additions/removals/content-size/mtime/attribute changes.
$inventoryRoots = @($source)
$dartMetadata = Join-Path $Package 'dart-sdk-path.txt'
if (Test-Path $dartMetadata) {
    $nativeDart = (Get-Content $dartMetadata -Raw).Trim()
    if ($nativeDart -and -not $nativeDart.StartsWith($source)) { $inventoryRoots += $nativeDart }
}
function Inventory {
    @(Get-ChildItem $inventoryRoots -Recurse -Force | Sort-Object FullName | ForEach-Object {
        "$($_.FullName)|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)|$($_.Attributes)"
    }) -join "
"
}
$before = Inventory
try {
    Invoke-MonitoredCommand -Stage flutter-early-version -FilePath "$Package\flutter\bin\flutter.bat" -ArgumentList @('--version') -DiagnosticDirectory $diagnostics -TimeoutSeconds 240
    if (-not (Test-Path "$root\.ready")) { throw 'Facade initialization marker missing' }
    if ((Get-Item "$root\bin\cache").Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Mutable cache must not be a junction' }
    if (-not ((Get-Item "$root\bin\cache\dart-sdk").Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Dart SDK must remain linked, not copied' }
    Invoke-MonitoredCommand -Stage flutter-nested-version -FilePath "$root\bin\flutter.bat" -ArgumentList @('--version') -DiagnosticDirectory $diagnostics -TimeoutSeconds 120
    Invoke-MonitoredCommand -Stage dart-early-version -FilePath "$root\bin\dart.bat" -ArgumentList @('--version') -DiagnosticDirectory $diagnostics -TimeoutSeconds 120
    Invoke-MonitoredCommand -Stage flutter-cache-write -FilePath "$root\bin\flutter.bat" -ArgumentList @('precache', '--windows') -DiagnosticDirectory $diagnostics -TimeoutSeconds 600
    $dartBytes = [IO.File]::ReadAllBytes("$root\bin\cache\dart-sdk\bin\dart.exe")
    $peOffset = [BitConverter]::ToInt32($dartBytes, 0x3c)
    $machine = [BitConverter]::ToUInt16($dartBytes, $peOffset + 4)
    $expected = if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq 'Arm64') { 0xaa64 } else { 0x8664 }
    if ($machine -ne $expected) { throw ('Dart architecture mismatch: expected {0:X}, got {1:X}' -f $expected, $machine) }
    Write-Host 'PASS: native Dart architecture, facade reuse, nested launcher, Windows precache'
} finally {
    if ($before -cne (Inventory)) { throw 'Immutable Flutter source metadata changed during smoke tests' }
    Write-Host 'PASS: immutable source inventory unchanged'
}
