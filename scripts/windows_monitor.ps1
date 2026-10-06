# Requires PowerShell 7 (the GitHub Actions pwsh shell).
# Commands run in a child pwsh so .bat/.cmd shims retain PowerShell's argument
# semantics. Output goes to independent files, never a blocking parent pipeline.

function Write-MonitorSnapshot {
    param([int]$RootId, [string]$Path, [hashtable]$Previous)
    try {
        $all = @(Get-CimInstance Win32_Process -OperationTimeoutSec 5 -ErrorAction Stop)
        $ids = [Collections.Generic.HashSet[int]]::new()
        [void]$ids.Add($RootId)
        do {
            $added = $false
            foreach ($item in $all) {
                if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $added = $true }
            }
        } while ($added)
        $rows = @(
            foreach ($item in $all) {
                if (-not $ids.Contains([int]$item.ProcessId)) { continue }
                $key = "$($item.ProcessId):$($item.CreationDate)"
                $cpu = ([double]$item.KernelModeTime + [double]$item.UserModeTime) / 1e7
                $delta = if ($Previous.ContainsKey($key)) { $cpu - $Previous[$key] } else { $null }
                $Previous[$key] = $cpu
                [ordered]@{ pid = $item.ProcessId; parent = $item.ParentProcessId
                    created = $item.CreationDate; name = $item.Name; commandLine = $item.CommandLine
                    cpuSeconds = $cpu; cpuDeltaSeconds = $delta; workingSetBytes = $item.WorkingSetSize }
            }
        )
        $os = Get-CimInstance Win32_OperatingSystem -OperationTimeoutSec 5 -ErrorAction Stop
        $disks = @(Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' -OperationTimeoutSec 5 -ErrorAction Stop |
            Select-Object DeviceID, FreeSpace, Size)
        [Console]::WriteLine(("[snapshot] availableRamMiB={0:N0} disks={1}" -f ($os.FreePhysicalMemory / 1024), (($disks | ForEach-Object { "$($_.DeviceID):free=$($_.FreeSpace)" }) -join ",")))
        foreach ($row in $rows) {
            [Console]::WriteLine("[process] pid=$($row.pid) parent=$($row.parent) cpuDeltaSeconds=$($row.cpuDeltaSeconds) workingSetBytes=$($row.workingSetBytes) command=$($row.commandLine)")
        }
        @{ timestamp = [DateTime]::UtcNow.ToString('o'); processes = $rows
            availableRamBytes = [long]$os.FreePhysicalMemory * 1024; disks = $disks } |
            ConvertTo-Json -Depth 6 -Compress | Add-Content -LiteralPath $Path
    } catch {
        @{ timestamp = [DateTime]::UtcNow.ToString('o'); snapshotError = "$_" } |
            ConvertTo-Json -Compress | Add-Content -LiteralPath $Path
    }
}

function Invoke-MonitoredCommand {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Stage,
        [Parameter(Mandatory)][string]$FilePath,
        [string[]]$ArgumentList = @(),
        [Parameter(Mandatory)][string]$DiagnosticDirectory,
        [Parameter(Mandatory)][ValidateRange(1,86400)][int]$TimeoutSeconds,
        [ValidateRange(1,300)][int]$HeartbeatSeconds = 30,
        [ValidateRange(256,65536)][int]$ForwardCharacters = 8192
    )
    $directory = [IO.Path]::GetFullPath($DiagnosticDirectory)
    [void][IO.Directory]::CreateDirectory($directory)
    $name = ($Stage -replace '[^a-zA-Z0-9_-]', '_') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8)
    $prefix = Join-Path $directory $name
    $stdout = "$prefix.stdout.log"
    $stderr = "$prefix.stderr.log"
    $events = "$prefix.events.log"
    $snapshot = "$prefix.processes.jsonl"
    # JSON + encoded PowerShell avoids shell interpolation and Start-Process's
    # lossy ArgumentList quoting (including paths with spaces and empty args).
    $payload = @{ file = $FilePath; arguments = @($ArgumentList) } | ConvertTo-Json -Compress
    $payload64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($payload))
    $child = @"
`$ErrorActionPreference = 'Stop'
`$PSNativeCommandUseErrorActionPreference = `$false
try {
    `$spec = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$payload64')) | ConvertFrom-Json
    `$arguments = @(
        foreach (`$argument in `$spec.arguments) { [string]`$argument }
    )
    & `$spec.file @arguments
    if (`$null -ne `$LASTEXITCODE) { exit `$LASTEXITCODE }
    exit 0
} catch { [Console]::Error.WriteLine(`$_); exit 1 }
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($child))
    $process = $null
    $sampler = $null
    $readers = @()
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $nextHeartbeat = 0.0
    $failure = $null
    $exitCode = $null
    $timedOut = $false
    $helper = $PSCommandPath
    $snapshotRequest = "$prefix.snapshot-request"
    $snapshotDone = "$prefix.snapshot-done"
    try {
        $outputFiles = @()
        $copies = @()
        foreach ($path in @($stdout,$stderr)) {
            $outputFiles += [IO.FileStream]::new($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite,1,$true)
        }
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = (Get-Process -Id $PID).Path
        $info.UseShellExecute = $false
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.WorkingDirectory = (Get-Location).Path
        foreach ($argument in @('-NoLogo','-NoProfile','-NonInteractive','-EncodedCommand',$encoded)) { $info.ArgumentList.Add($argument) }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $info
        [void]$process.Start()
        # Raw async byte copies handle newline-free output without line-event buffering.
        $copies += $process.StandardOutput.BaseStream.CopyToAsync($outputFiles[0])
        $copies += $process.StandardError.BaseStream.CopyToAsync($outputFiles[1])
        # CIM queries are isolated: slow WMI cannot delay heartbeats or deadlines.
        if ($IsWindows) {
            $samplerSpec = @{ helper = $helper; rootId = $process.Id; path = $snapshot
                interval = $HeartbeatSeconds; request = $snapshotRequest; done = $snapshotDone } | ConvertTo-Json -Compress
            $sampler64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($samplerSpec))
            $samplerCode = @"
`$s = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$sampler64')) | ConvertFrom-Json
. `$s.helper
`$previous = @{}
`$next = [DateTime]::UtcNow
while (Get-Process -Id `$s.rootId -ErrorAction SilentlyContinue) {
    `$requested = Test-Path -LiteralPath `$s.request
    if (`$requested -or [DateTime]::UtcNow -ge `$next) {
        Write-MonitorSnapshot -RootId `$s.rootId -Path `$s.path -Previous `$previous
        `$next = [DateTime]::UtcNow.AddSeconds(`$s.interval)
        if (`$requested) { Set-Content -LiteralPath `$s.done -Value done; break }
    }
    Start-Sleep -Milliseconds 250
}
"@
            $samplerEncoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($samplerCode))
            $sampler = Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList @('-NoProfile','-NonInteractive','-EncodedCommand',$samplerEncoded) -RedirectStandardOutput "$prefix.snapshot-console.log" -RedirectStandardError "$prefix.snapshot-errors.log" -PassThru
        }
        $forwardPaths = @($stdout,$stderr)
        if ($sampler) { $forwardPaths += "$prefix.snapshot-console.log" }
        foreach ($path in $forwardPaths) {
            $stream = [IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite)
            $readers += [IO.StreamReader]::new($stream,[Text.Encoding]::UTF8,$true,4096)
        }
        $buffer = [char[]]::new($ForwardCharacters)
        while ($true) {
            if ($clock.Elapsed.TotalSeconds -ge $nextHeartbeat) {
                $line = "[$([DateTime]::UtcNow.ToString('o'))][monitor:$Stage] heartbeat elapsed=$([int]$clock.Elapsed.TotalSeconds)s pid=$($process.Id) stdoutBytes=$((Get-Item -LiteralPath $stdout).Length) stderrBytes=$((Get-Item -LiteralPath $stderr).Length)"
                Write-Host $line
                Add-Content -LiteralPath $events -Value $line
                $nextHeartbeat = $clock.Elapsed.TotalSeconds + $HeartbeatSeconds
            }
            $process.Refresh()
            if ($process.HasExited) { $exitCode = $process.ExitCode; break }
            if ($clock.Elapsed.TotalSeconds -ge $TimeoutSeconds) {
                $timedOut = $true
                throw "Stage '$Stage' timed out after $TimeoutSeconds seconds; logs: $prefix.*"
            }
            # At most one bounded chunk per stream per tick, even for an
            # unbroken line or a flood. Files preserve all bytes, console does not.
            for ($index = 0; $index -lt $readers.Count; $index++) {
                $count = $readers[$index].Read($buffer,0,$buffer.Length)
                if ($count -gt 0) { Write-Host "[$Stage/$(@('stdout','stderr','snapshot')[$index])] $([string]::new($buffer,0,$count))" }
            }
            Start-Sleep -Milliseconds 250
        }
        # Bounded final pending chunk; full remaining output stays in artifacts.
        foreach ($reader in $readers) {
            $count = $reader.Read($buffer,0,$buffer.Length)
            if ($count -gt 0) { Write-Host "[$Stage/final] $([string]::new($buffer,0,$count))" }
        }
        if ($exitCode -ne 0) { throw "Stage '$Stage' failed with exit code $exitCode; logs: $prefix.*" }
    } catch { $failure = $_ }
    finally {
        # Do not let cleanup or diagnostics replace the original exit/timeout.
        if ($timedOut -and $sampler) {
            try {
                Set-Content -LiteralPath $snapshotRequest -Value requested
                $grace = [Diagnostics.Stopwatch]::StartNew()
                while ($grace.Elapsed.TotalSeconds -lt 2 -and -not (Test-Path -LiteralPath $snapshotDone)) { Start-Sleep -Milliseconds 100 }
            } catch { Write-Warning "Final snapshot request failed: $_" }
        }
        if ($process) {
            try {
                if (-not $process.HasExited) {
                    $process.Kill($true)
                    if (-not $process.WaitForExit(10000)) { Write-Warning "Process tree did not exit within cleanup grace period" }
                }
            } catch { Write-Warning "Process tree cleanup failed: $_" }
        }
        if ($sampler) {
            try {
                if (-not $sampler.HasExited) { $sampler.Kill($true); [void]$sampler.WaitForExit(2000) }
                $sampler.Dispose()
            }
            catch { Write-Warning "Snapshot worker cleanup failed: $_" }
        }
        try {
            if ($copies.Count -gt 0 -and -not [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]$copies,2000)) {
                if (-not $failure) { $failure = 'Output drain timed out; a descendant may still hold the output pipe' }
                Write-Warning 'Output drain timed out after 2 seconds'
            }
        } catch {
            if (-not $failure) { $failure = $_ }
            Write-Warning "Output capture failed: $_"
        }
        # Forward one more bounded chunk after asynchronous capture has drained.
        foreach ($reader in $readers) {
            try {
                $count = $reader.Read($buffer,0,$buffer.Length)
                if ($count -gt 0) { Write-Host "[$Stage/drained] $([string]::new($buffer,0,$count))" }
            } catch { Write-Warning "Final output forwarding failed: $_" }
        }
        foreach ($file in $outputFiles) { try { $file.Dispose() } catch { Write-Warning "Output file cleanup failed: $_" } }
        foreach ($reader in $readers) { try { $reader.Dispose() } catch { Write-Warning "Reader cleanup failed: $_" } }
        try {
            @{ timestamp = [DateTime]::UtcNow.ToString('o'); stage = $Stage; command = $FilePath; arguments = $ArgumentList
                elapsedSeconds = $clock.Elapsed.TotalSeconds; exitCode = $exitCode
                timedOut = $timedOut; error = "$failure"; stdout = $stdout; stderr = $stderr } |
                ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$prefix.result.json"
        } catch { Write-Warning "Could not write stage result: $_" }
        if ($process) { $process.Dispose() }
    }
    if ($failure) { throw $failure }
}

function Write-FlutterPreflight {
    param([string]$DiagnosticDirectory)
    [void][IO.Directory]::CreateDirectory($DiagnosticDirectory)
    $path = Join-Path $DiagnosticDirectory 'flutter-preflight.txt'
    & {
        try {
            $command = Get-Command flutter -ErrorAction Stop
            "Resolved Flutter: $($command.Source)"
            $cache = Join-Path (Split-Path $command.Source -Parent) 'cache'
            foreach ($item in @((Split-Path $command.Source -Parent),$cache,(Join-Path $cache 'flutter.bat.lock'),(Join-Path $cache 'lockfile'))) {
                if (Test-Path -LiteralPath $item) {
                    Get-Item -Force -LiteralPath $item | Format-List FullName,Attributes,Length,CreationTimeUtc,LastWriteTimeUtc,LinkType,Target | Out-String
                    if ($IsWindows) { Get-Acl -LiteralPath $item | Format-List Path,Owner,AccessToString | Out-String }
                } else { "Missing: $item" }
            }
            # Never touch Flutter's locks or alter SDK permissions.
            $probe = Join-Path $cache ("rune-diagnostic-" + [Guid]::NewGuid().ToString('N') + '.tmp')
            $created = $false
            try {
                $file = [IO.File]::Open($probe,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
                $created = $true
                $file.Dispose()
                "Cache unique-file write probe: succeeded"
            } catch { "Cache unique-file write probe: failed: $_" }
            finally { if ($created) { Remove-Item -LiteralPath $probe -ErrorAction Continue } }
        } catch { "Flutter preflight failed: $_" }
    } | Out-File -LiteralPath $path
    Get-Content -LiteralPath $path | Write-Host
}
