# Package-owned writable facade; never chmod, hardlink, or copy the full SDK.
$ErrorActionPreference = 'Stop'
$package = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$source = (Get-Content (Join-Path $Package 'sdk-path.txt') -Raw).Trim()
$dartSdk = (Get-Content (Join-Path $package 'dart-sdk-path.txt') -Raw).Trim()
$architecture = (Get-Content (Join-Path $package 'architecture.txt') -Raw).Trim()
$dart = Join-Path $dartSdk 'bin\dart.exe'
$key = Split-Path $package -Leaf
$base = Join-Path $env:LOCALAPPDATA 'Rune\Flutter'
$root = Join-Path $base $key
# Upgrade commands can write through immutable source junctions.
# Conservative token guard also rejects these names as positional arguments.
if ($env:RUNE_DART_LAUNCH -ne '1' -and @($args | Where-Object { $_ -in @('upgrade', 'downgrade', 'channel') }).Count) {
    throw 'This Flutter SDK is managed by Nix; change the pinned package instead.'
}
New-Item -ItemType Directory -Force $base | Out-Null
$deadline = [DateTime]::UtcNow.AddMinutes(2)
$lock = $null
while (-not $lock) {
    try { $lock = [IO.File]::Open("$root.init-lock", 'OpenOrCreate', 'ReadWrite', 'None') }
    catch [IO.IOException] {
        if ([DateTime]::UtcNow -gt $deadline) { throw 'Timed out initializing Flutter facade' }
        Start-Sleep -Milliseconds 200
    }
}
function Copy-WritableFile($from, $to) {
    Copy-Item -LiteralPath $from -Destination $to
    (Get-Item -LiteralPath $to -Force).IsReadOnly = $false
}
function Copy-WritableTree($from, $to) {
    New-Item -ItemType Directory -Path $to | Out-Null
    foreach ($item in Get-ChildItem -LiteralPath $from -Force) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Unexpected source link: $($item.FullName)" }
        $dest = Join-Path $to $item.Name
        if ($item.PSIsContainer) { Copy-WritableTree $item.FullName $dest }
        else { Copy-WritableFile $item.FullName $dest }
    }
}
function Link-Directory($from, $to) {
    New-Item -ItemType Junction -Path $to -Target $from | Out-Null
}
try {
    if (-not (Test-Path "$root\.ready")) {
        # Do not recursively delete a partial tree containing Store junctions.
        if (Test-Path $root) { throw "Incomplete Flutter facade at $root; rename it before retrying." }
        New-Item -ItemType Directory $root | Out-Null
        foreach ($item in Get-ChildItem -LiteralPath $source -Force) {
            if ($item.Name -in @('bin', 'packages')) { continue }
            $dest = Join-Path $root $item.Name
            if ($item.PSIsContainer) { Link-Directory $item.FullName $dest }
            else { Copy-WritableFile $item.FullName $dest }
        }
        New-Item -ItemType Directory "$root\bin", "$root\bin\cache", "$root\packages" | Out-Null
        foreach ($item in Get-ChildItem "$source\bin" -Force) {
            if ($item.Name -eq 'cache') { continue }
            $dest = Join-Path "$root\bin" $item.Name
            if ($item.PSIsContainer) { Link-Directory $item.FullName $dest }
            else { Copy-WritableFile $item.FullName $dest }
        }
        foreach ($item in Get-ChildItem "$source\bin\cache" -Force) {
            if ($item.Name -in @('lockfile', 'flutter.bat.lock')) { continue }
            $dest = Join-Path "$root\bin\cache" $item.Name
            if ($item.Name -eq 'dart-sdk') { Link-Directory $dartSdk $dest }
            elseif ($item.PSIsContainer) { Copy-WritableTree $item.FullName $dest }
            else { Copy-WritableFile $item.FullName $dest }
        }
        foreach ($item in Get-ChildItem "$source\packages" -Directory -Force) {
            $dest = Join-Path "$root\packages" $item.Name
            # PubDependencies regenerates package_config.json and resolves tool deps.
            if ($item.Name -eq 'flutter_tools') { Copy-WritableTree $item.FullName $dest }
            else { Link-Directory $item.FullName $dest }
        }
        # Nested CMake invocations must also bypass upstream batch bootstrap.
        foreach ($name in @('flutter', 'dart')) {
            $batch = (Get-Content "$PSScriptRoot\$name.bat" -Raw).Replace('%~dp0flutter-launcher.ps1', "$PSScriptRoot\flutter-launcher.ps1")
            Set-Content "$root\bin\$name.bat" $batch -Encoding ASCII
        }
        Set-Content "$root\.ready.tmp" $source
        Move-Item "$root\.ready.tmp" "$root\.ready"
    }
    if ((Get-Content "$root\.ready" -Raw).Trim() -ne $source) { throw "Flutter facade identity mismatch: $root" }
} finally { $lock.Dispose() }
$env:FLUTTER_ROOT = $root
$env:GIT_OPTIONAL_LOCKS = '0'
if (-not $env:PUB_CACHE) { $env:PUB_CACHE = Join-Path $env:LOCALAPPDATA 'Pub\Cache' }
Remove-Item Env:FLUTTER_ALREADY_LOCKED -ErrorAction SilentlyContinue
# ARM64 cannot execute the archive's x64 tool snapshot. Resolve dependencies
# only in the writable facade, under the same initialization lock.
if ($architecture -eq 'arm64') {
    $toolLock = [IO.File]::Open("$root.tool-lock", 'OpenOrCreate', 'ReadWrite', 'None')
    try {
        if (-not (Test-Path "$root\.native-tool-ready")) {
            Push-Location "$root\packages\flutter_tools"
            try {
                & $dart pub get
                if ($LASTEXITCODE -ne 0) { throw 'Native Flutter tool dependency resolution failed' }
            } finally { Pop-Location }
            Set-Content "$root\.native-tool-ready" $dartSdk
        }
    } finally { $toolLock.Dispose() }
}
if ($env:RUNE_DART_LAUNCH -eq '1') { & $dart @args }
elseif ($architecture -eq 'arm64') {
    & $dart "--packages=$root\packages\flutter_tools\.dart_tool\package_config.json" "$root\packages\flutter_tools\bin\flutter_tools.dart" @args
}
else { & $dart "$root\bin\cache\flutter_tools.snapshot" @args }
exit $LASTEXITCODE
