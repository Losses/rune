$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Add-Type -AssemblyName System.IO.Compression.FileSystem
$root = [IO.Path]::GetFullPath($env:out)
$work = Join-Path $root '.unpack'
New-Item -ItemType Directory -Force $work | Out-Null
$env:TMP = $work
$env:TEMP = $work
function Get-Payloads([string]$Records) {
    foreach ($line in ($Records -split "`n")) {
        $fields = $line -split "`t", 2
        if ($fields.Count -ne 2) { throw "Invalid payload record: $line" }
        [PSCustomObject]@{ path = $fields[0]; fileName = $fields[1] }
    }
}

function Get-OutputPath([string]$Relative) {
    $path = [IO.Path]::GetFullPath((Join-Path $root $Relative))
    if (-not $path.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Payload path escapes output: $Relative"
    }
    return $path
}
function Get-MsiRows($Database, [string]$Table, [string[]]$Columns) {
    $fields = ($Columns | ForEach-Object { '`' + $_ + '`' }) -join ', '
    $view = $Database.OpenView("SELECT $fields FROM ``$Table``")
    [void]$view.Execute()
    try {
        while ($record = $view.Fetch()) {
            try {
                $row = @{}
                for ($i = 0; $i -lt $Columns.Count; $i++) { $row[$Columns[$i]] = $record.StringData($i + 1) }
                $row
            } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($record) }
        }
    } finally {
        [void]$view.Close()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($view)
    }
}
function Get-MsiDirectory($Id, $Directories, $Resolved) {
    if ($Resolved.ContainsKey($Id)) { return $Resolved[$Id] }
    $entry = $Directories[$Id]
    if (-not $entry) { throw "Missing MSI directory: $Id" }
    $parent = Get-MsiDirectory $entry.Directory_Parent $Directories $Resolved
    $name = ($entry.DefaultDir -split ':')[0] -replace '^.*\|', ''
    $path = if ($name -eq '.') { $parent } else { Join-Path $parent $name }
    $Resolved[$Id] = $path
    return $path
}

foreach ($payload in (Get-Payloads $env:vsixPayloads)) {
    Write-Host "Unpacking $($payload.fileName)"
    $zip = [IO.Compression.ZipFile]::OpenRead($payload.path)
    try {
        foreach ($entry in $zip.Entries) {
            if (-not $entry.FullName.StartsWith('Contents/') -or -not $entry.Name) { continue }
            $relative = [Uri]::UnescapeDataString($entry.FullName.Substring(9)).Replace('/', '\')
            $dest = Get-OutputPath $relative
            New-Item -ItemType Directory -Force ([IO.Path]::GetDirectoryName($dest)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dest, $true)
        }
    } finally { $zip.Dispose() }
}

$staging = Join-Path $work 'sdk'
New-Item -ItemType Directory $staging | Out-Null
foreach ($payload in (Get-Payloads $env:sdkPayloads)) { Copy-Item -LiteralPath $payload.path -Destination (Join-Path $staging $payload.fileName) }
$installer = New-Object -ComObject WindowsInstaller.Installer
try {
    foreach ($msi in (Get-ChildItem $staging -Filter '*.msi' | Sort-Object Name)) {
        Write-Host "Unpacking $($msi.Name)"
        $db = $installer.OpenDatabase($msi.FullName, 0)
        try {
            $cabDir = Join-Path $work 'cab'
            New-Item -ItemType Directory $cabDir | Out-Null
            foreach ($media in (Get-MsiRows $db 'Media' @('Cabinet'))) {
                if (-not $media.Cabinet) { continue }
                if ($media.Cabinet.StartsWith('#')) { throw "Embedded cabinet not supported: $($msi.Name)" }
                $cab = Join-Path $staging $media.Cabinet
                $process = New-Object Diagnostics.Process
                $process.StartInfo.FileName = 'C:\Windows\System32\expand.exe'
                $process.StartInfo.Arguments = "-F:* `"$cab`" `"$cabDir`""
                $process.StartInfo.UseShellExecute = $false
                $process.StartInfo.RedirectStandardOutput = $true
                [void]$process.Start()
                $output = $process.StandardOutput.ReadToEndAsync()
                if (-not $process.WaitForExit(120000)) {
                    $process.Kill()
                    $process.WaitForExit()
                    throw "Cabinet extraction timed out: $cab"
                }
                if ($process.ExitCode -ne 0) { throw "Cabinet extraction failed: $cab ($($process.ExitCode)): $($output.Result)" }
                $process.Dispose()
            }
            $directories = @{}
            foreach ($row in (Get-MsiRows $db 'Directory' @('Directory', 'Directory_Parent', 'DefaultDir'))) { $directories[$row.Directory] = $row }
            # Redirect standard installation roots; never execute MSI actions or write the registry.
            $resolved = @{ TARGETDIR = '.'; ProgramFilesFolder = '.'; ProgramFiles64Folder = '.' }
            $components = @{}
            foreach ($row in (Get-MsiRows $db 'Component' @('Component', 'Directory_'))) { $components[$row.Component] = $row.Directory_ }
            foreach ($row in (Get-MsiRows $db 'File' @('File', 'Component_', 'FileName'))) {
                $directory = Get-MsiDirectory $components[$row.Component_] $directories $resolved
                $name = $row.FileName -replace '^.*\|', ''
                $dest = Get-OutputPath (Join-Path $directory $name)
                New-Item -ItemType Directory -Force ([IO.Path]::GetDirectoryName($dest)) | Out-Null
                Copy-Item -LiteralPath (Join-Path $cabDir $row.File) -Destination $dest -Force
            }
            Remove-Item -Recurse -Force $cabDir
        } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($db) }
    }
} finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($installer) }

foreach ($arch in @('x64', 'arm64')) {
    foreach ($relative in @(
        "VC/Tools/MSVC/$env:vcVersion/bin/Hostx64/$arch/cl.exe",
        "VC/Tools/MSVC/$env:vcVersion/bin/Hostx64/$arch/link.exe",
        "VC/Tools/MSVC/$env:vcVersion/lib/$arch/libcpmt.lib",
        "VC/Tools/MSVC/$env:vcVersion/lib/$arch/oldnames.lib",
        "VC/Tools/MSVC/$env:vcVersion/lib/$arch/msvcrt.lib",
        "VC/Tools/MSVC/$env:vcVersion/include/vector",
        "VC/Tools/MSVC/$env:vcVersion/atlmfc/include/atlbase.h",
        "VC/Tools/MSVC/$env:vcVersion/atlmfc/lib/$arch/atls.lib",
        "Windows Kits/10/Include/$env:sdkVersion/um/Windows.h",
        "Windows Kits/10/Include/$env:sdkVersion/shared/sdkddkver.h",
        "Windows Kits/10/Include/$env:sdkVersion/ucrt/stdio.h",
        "Windows Kits/10/Lib/$env:sdkVersion/um/$arch/kernel32.lib",
        "Windows Kits/10/Lib/$env:sdkVersion/ucrt/$arch/ucrt.lib",
        "Windows Kits/10/bin/$env:sdkVersion/x64/rc.exe"
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $root $relative))) { throw "Incomplete toolchain: $relative" }
    }
}
Remove-Item -Recurse -Force $work
