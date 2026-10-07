param([Parameter(Mandatory)][ValidateSet('x64','arm64')][string]$Arch)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$path = Join-Path $PSScriptRoot "../build/windows/$Arch/runner/Release/rune.msix"
$zip = [IO.Compression.ZipFile]::OpenRead($path)
try {
    $entry = $zip.GetEntry('AppxManifest.xml')
    if (-not $entry) { throw 'MSIX manifest missing' }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    if ($manifest.Package.Identity.ProcessorArchitecture -ne $Arch) { throw 'MSIX architecture mismatch' }
    foreach ($name in @('rune.exe', 'hub.dll', 'flutter_windows.dll')) {
        $matches = @($zip.Entries | Where-Object { $_.Name -eq $name })
        if ($matches.Count -ne 1) { throw "Expected one $name in MSIX" }
        $stream = $matches[0].Open()
        $memory = [IO.MemoryStream]::new()
        try { $stream.CopyTo($memory); $bytes = $memory.ToArray() } finally { $stream.Dispose(); $memory.Dispose() }
        if ($bytes.Length -lt 64 -or $bytes[0] -ne 0x4d -or $bytes[1] -ne 0x5a) { throw "$name is not a PE executable" }
        $offset = [BitConverter]::ToInt32($bytes, 0x3c)
        if ($offset -lt 0 -or $offset -gt ($bytes.Length - 6) -or [BitConverter]::ToUInt32($bytes, $offset) -ne 0x4550) { throw "$name has an invalid PE header" }
        $machine = [BitConverter]::ToUInt16($bytes, $offset + 4)
        $expected = if ($Arch -eq 'arm64') { 0xaa64 } else { 0x8664 }
        if ($machine -ne $expected) { throw "$name has wrong MSIX payload architecture" }
    }
    if (-not ($zip.Entries | Where-Object { $_.FullName -match 'data/flutter_assets/' })) { throw 'MSIX Flutter assets missing' }
    Write-Host "PASS: MSIX $Arch manifest, executable, native libraries and Flutter assets"
} finally { $zip.Dispose() }
