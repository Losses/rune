param([Parameter(Mandatory)][string]$Package)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\windows_monitor.ps1"
$diagnostics = 'build/diagnostics/flutter-smoke'
$source = (Get-Content (Join-Path $Package 'sdk-path.txt') -Raw).Trim()
$root = Join-Path $env:LOCALAPPDATA ('Rune\Flutter\' + (Split-Path $Package -Leaf))
# Metadata inventory detects additions/removals/content-size/mtime/attribute changes.
function Inventory {
    @(Get-ChildItem $source -Recurse -Force | Sort-Object FullName | ForEach-Object {
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
    Write-Host 'PASS: direct snapshot, facade reuse, nested launcher, Dart, Windows precache'
} finally {
    if ($before -cne (Inventory)) { throw 'Immutable Flutter source metadata changed during smoke tests' }
    Write-Host 'PASS: immutable source inventory unchanged'
}
