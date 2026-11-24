# Create a private Flutter 3.47.4 SDK with Flutter PR #193372 applied.
# This never edits the source SDK or global Git/Flutter configuration.
param(
  [Parameter(Mandatory)][string]$SourceSdk,
  [Parameter(Mandatory)][string]$DestinationSdk,
  [switch]$CheckOnly
)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class RuneAxSdkPaths {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern uint GetLongPathName(string path, StringBuilder result, uint capacity);
  public static string LongPath(string path) {
    var full=System.IO.Path.GetFullPath(path);
    var buffer=new StringBuilder(32768);
    uint length=GetLongPathName(full, buffer, (uint)buffer.Capacity);
    if (length==0 || length>=buffer.Capacity) throw new System.ComponentModel.Win32Exception();
    return buffer.ToString();
  }
}
'@
$source = [RuneAxSdkPaths]::LongPath((Resolve-Path -LiteralPath $SourceSdk).Path).TrimEnd('\')
$destinationFull = [IO.Path]::GetFullPath($DestinationSdk).TrimEnd('\')
$destinationParent = Split-Path -Parent $destinationFull
$destinationName = Split-Path -Leaf $destinationFull
if (-not (Test-Path -LiteralPath $destinationParent -PathType Container)) { throw 'Destination parent must already exist.' }
$destination = Join-Path ([RuneAxSdkPaths]::LongPath((Resolve-Path -LiteralPath $destinationParent).Path)) $destinationName
if (Test-Path -LiteralPath $destination) { throw 'Destination must not already exist.' }
if ($destination.StartsWith($source + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Destination cannot be inside source SDK.' }
$patches = @(
    @{ Path = (Join-Path $PSScriptRoot 'patches\flutter-193372.patch'); Hash = 'ba4b9bdc82199388cb05905d2c3770d585de53d3148df1e06c5000d79cf66b48' },
    @{ Path = (Join-Path $PSScriptRoot 'patches\flutter-windows-ime-geometry.patch'); Hash = '0f7d9424414429e9ead93a2f524c39c3a11715b37cd0b5f7fb830474264cec3a' }
)
foreach ($patch in $patches) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $patch.Path).Hash.ToLowerInvariant() -ne $patch.Hash) { throw "Flutter patch checksum mismatch: $($patch.Path)" }
}
if (-not (Test-Path -LiteralPath (Join-Path $source 'bin\flutter.bat'))) { throw 'Source is not a Flutter SDK.' }
$gitSource = $source.Replace('\', '/')
$revision = (& git -c "safe.directory=$gitSource" -C $source rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $revision -ne '9584c6713b324636289d067944a46fd6b49df14b') { throw "Expected Flutter 3.47.4 revision; got $revision." }
& git -c "safe.directory=$gitSource" -C $source diff --quiet HEAD -- packages/flutter/lib/src/semantics/semantics.dart packages/flutter/test/semantics/semantics_update_test.dart packages/flutter/lib/src/services/text_input.dart
if ($LASTEXITCODE -ne 0) { throw 'Source SDK already has modifications to the patch target files.' }
if (Test-Path -LiteralPath (Join-Path $source 'packages/flutter/test/widgets/windows_ime_geometry_compat_test.dart')) { throw 'Source SDK already contains the IME regression test.' }
if ($CheckOnly) { Write-Output "SDK preconditions passed: $source -> $destination"; exit 0 }
& robocopy $source $destination /E /MT:8 /NFL /NDL /NJH /NJS /NP
if ($LASTEXITCODE -ge 8) { throw "SDK copy failed with robocopy exit $LASTEXITCODE; inspect destination before retrying." }
$gitDestination = $destination.Replace('\', '/')
foreach ($patch in $patches) {
    & git -c "safe.directory=$gitDestination" -C $destination apply --check $patch.Path
    if ($LASTEXITCODE -ne 0) { throw "Patch does not apply cleanly to SDK copy: $($patch.Path)" }
    & git -c "safe.directory=$gitDestination" -C $destination apply $patch.Path
    if ($LASTEXITCODE -ne 0) { throw "Patch application failed in SDK copy: $($patch.Path)" }
}
Write-Output "Patched Flutter SDK: $destination"
Write-Output "Flutter revision: $revision"
foreach ($patch in $patches) { Write-Output "Patch: $($patch.Path); SHA256: $($patch.Hash)" }
