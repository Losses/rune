# Run on Windows: powershell -File generate-msvc.ps1 [-Toolset 14.44] [-Cache <dir>]
param(
    [string]$Toolset = '',
    [string]$ManifestSha256 = '',
    [string]$Cache = (Join-Path $PSScriptRoot '.msvc-downloads'),
    [string]$Output = (Join-Path $PSScriptRoot 'msvc-packages.nix')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
New-Item -ItemType Directory -Force $Cache | Out-Null
$Cache = (Resolve-Path $Cache).Path

function Get-Download($Url, $Name, $Hash = '') {
    $path = Join-Path $Cache $Name
    if (-not $Hash -or -not (Test-Path $path) -or (Get-FileHash $path -Algorithm SHA256).Hash -ne $Hash) {
        & curl.exe --fail --location --silent --show-error --max-time 180 --output $path $Url
        if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
    }
    if ($Hash -and (Get-FileHash $path -Algorithm SHA256).Hash -ne $Hash) { throw "SHA256 mismatch: $Name" }
    return $path
}
function Get-Payload($Payload) {
    Get-Download $Payload.url ([IO.Path]::GetFileName($Payload.fileName)) $Payload.sha256
}
function Quote-Nix([string]$Value) {
    '"' + $Value.Replace('\', '\\').Replace('"', '\"').Replace('${', '\${') + '"'
}

$channelUrl = 'https://aka.ms/vs/17/release/channel'
$channel = Get-Content -Raw (Get-Download $channelUrl 'channel.json') | ConvertFrom-Json
$manifestPayload = @($channel.channelItems | Where-Object id -eq 'Microsoft.VisualStudio.Manifests.VisualStudio')[0].payloads[0]
$manifestHash = if ($ManifestSha256) { $ManifestSha256 } else { $manifestPayload.sha256 }
$manifest = Get-Content -Raw (Get-Download $manifestPayload.url $manifestPayload.fileName $manifestHash) | ConvertFrom-Json
$packages = @{}
foreach ($p in $manifest.packages) {
    if (-not $p.language -or $p.language -eq 'en-US') { $packages[$p.id] = $p }
}
$prefixes = @($packages.Keys | Where-Object { $_ -match '^Microsoft.VC.(14\.\d+\.17\.\d+)\.Tools.HostX64.TargetX64.base$' } |
    ForEach-Object { $_ -replace '\.Tools.HostX64.TargetX64.base$', '' })
if ($Toolset) { $prefixes = @($prefixes | Where-Object { $_.StartsWith("Microsoft.VC.$Toolset.") }) }
$prefix = $prefixes | Sort-Object { [version]($_ -replace '^Microsoft.VC.', '') } | Select-Object -Last 1
if (-not $prefix -or [version]($prefix -replace '^Microsoft.VC.', '') -lt [version]'14.41.0.0') {
    throw 'No matching MSVC toolset >= 14.41 in the channel'
}
$vsix = @()
foreach ($suffix in @('Tools.HostX64.TargetX64.base', 'Tools.HostX64.TargetX64.Res.base',
    'Tools.HostX64.TargetARM64.base', 'Tools.HostX64.TargetARM64.Res.base',
    'CRT.Headers.base', 'CRT.x64.Desktop.base', 'CRT.x64.Store.base',
    'CRT.ARM64.Desktop.base', 'CRT.ARM64.Store.base',
    'ATL.Headers.base', 'ATL.X64.base', 'ATL.ARM64.base')) {
    $id = "$prefix.$suffix"
    if (-not $packages.ContainsKey($id)) { throw "Missing package: $id" }
    $vsix += $packages[$id].payloads
}
# Package servicing versions differ from the actual VC/Tools/MSVC directory.
$zip = [IO.Compression.ZipFile]::OpenRead((Get-Payload $vsix[0]))
try {
    $versions = @($zip.Entries | ForEach-Object {
        if ($_.FullName -match '^Contents/VC/Tools/MSVC/([^/]+)/') { $matches[1] }
    } | Sort-Object -Unique)
    if ($versions.Count -ne 1) { throw 'Ambiguous MSVC directory version' }
    $vcVersion = $versions[0]
} finally { $zip.Dispose() }
$sdk = $packages.Values | Where-Object { $_.id -match '^Win11SDK_10\.0\.\d+$' } |
    Sort-Object { [version]($_.id -replace '^Win11SDK_', '') } | Select-Object -Last 1
if (-not $sdk) { throw 'No Windows SDK in the channel' }
$sdkVersion = ($sdk.id -replace '^Win11SDK_', '') + '.0'
$sdkPayloads = @{}
foreach ($p in $sdk.payloads) { $sdkPayloads[[IO.Path]::GetFileName($p.fileName)] = $p }
$msiNames = @(
    'Windows SDK Desktop Headers x86-x86_en-us.msi',
    'Windows SDK Desktop Headers x64-x86_en-us.msi',
    'Windows SDK Desktop Headers arm64-x86_en-us.msi',
    'Windows SDK Desktop Libs x64-x86_en-us.msi',
    'Windows SDK Desktop Libs arm64-x86_en-us.msi',
    'Windows SDK for Windows Store Apps Headers-x86_en-us.msi',
    'Windows SDK for Windows Store Apps Headers OnecoreUap-x86_en-us.msi',
    'Windows SDK OnecoreUap Headers x86-x86_en-us.msi',
    'Windows SDK OnecoreUap Headers x64-x86_en-us.msi',
    'Windows SDK OnecoreUap Headers arm64-x86_en-us.msi',
    'Windows SDK for Windows Store Apps Libs-x86_en-us.msi',
    'Windows SDK for Windows Store Apps Tools-x86_en-us.msi',
    'Universal CRT Headers Libraries and Sources-x86_en-us.msi'
)
$selected = @{}
$installer = New-Object -ComObject WindowsInstaller.Installer
foreach ($name in $msiNames) {
    if (-not $sdkPayloads.ContainsKey($name)) { throw "Missing SDK MSI: $name" }
    $payload = $sdkPayloads[$name]
    $selected[$name] = $payload
    $db = $installer.OpenDatabase((Get-Payload $payload), 0)
    $view = $db.OpenView('SELECT `Cabinet` FROM `Media`')
    $view.Execute()
    while ($record = $view.Fetch()) {
        $cab = $record.StringData(1)
        if ($cab -and -not $cab.StartsWith('#')) {
            if (-not $sdkPayloads.ContainsKey($cab)) { throw "Cabinet absent from manifest: $cab" }
            $selected[$cab] = $sdkPayloads[$cab]
        }
    }
    $view.Close()
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($db)
}
[void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer)
$lines = @(
    '# Generated by generate-msvc.ps1; do not edit payloads by hand.',
    "# Channel: $channelUrl",
    "# Manifest: $($manifestPayload.url)",
    "# Channel-published manifest SHA256: $($manifestPayload.sha256)",
    "# Verified manifest SHA256: $manifestHash",
    "# Toolset package family: $prefix; compiler package version: $($packages["$prefix.Tools.HostX64.TargetX64.base"].version)",
    "# SDK package: $($sdk.id) $($sdk.version)",
    '{', "  vcVersion = $(Quote-Nix $vcVersion);", "  sdkVersion = $(Quote-Nix $sdkVersion);"
)
foreach ($group in @('vsix', 'sdk')) {
    $lines += "  $group = ["
    $payloads = if ($group -eq 'vsix') { $vsix } else { $selected.Values }
    foreach ($p in ($payloads | Sort-Object fileName)) {
        if ($p.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or $p.url -notmatch '^https://') { throw 'Unpinned payload' }
        $lines += @('    {', "      url = $(Quote-Nix $p.url);", "      sha256 = $(Quote-Nix $p.sha256.ToLowerInvariant());",
            "      fileName = $(Quote-Nix ([IO.Path]::GetFileName($p.fileName)));", '    }')
    }
    $lines += '  ];'
}
$lines += '}'
[IO.File]::WriteAllText($Output, ($lines -join "`n") + "`n", (New-Object Text.UTF8Encoding($false)))
Write-Host "Generated $Output : MSVC $vcVersion / SDK $sdkVersion ($($vsix.Count) VSIX, $($selected.Count) SDK payloads)"
