# Run: pwsh -NoProfile -File scripts/test_windows_monitor.ps1
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows_monitor.ps1"
$root = Join-Path $PSScriptRoot "../build/diagnostics/tests/$([Guid]::NewGuid().ToString('N'))"
[void][IO.Directory]::CreateDirectory($root)
$pwsh = (Get-Process -Id $PID).Path
function Assert($condition, $message) { if (-not $condition) { throw $message } }
function Run-Mock($stage, $source, $timeout = 20) {
    $script = Join-Path $root "$stage.ps1"
    Set-Content -LiteralPath $script -Value $source
    Invoke-MonitoredCommand -Stage $stage -FilePath $pwsh -ArgumentList @('-NoProfile','-File',$script) -DiagnosticDirectory $root -TimeoutSeconds $timeout -HeartbeatSeconds 1 -ForwardCharacters 256
}
function Result($stage) {
    Get-Content -Raw (Get-ChildItem $root -Filter "$stage-*.result.json" | Select-Object -Last 1).FullName | ConvertFrom-Json
}
Run-Mock silence 'Start-Sleep -Seconds 3'
$heartbeats = @(Get-Content (Get-ChildItem $root -Filter 'silence-*.events.log').FullName)
Assert ($heartbeats.Count -ge 3) 'Silent command did not receive independent heartbeats'
Run-Mock streams @'
for ($i = 0; $i -lt 100; $i++) {
    [Console]::Out.WriteLine("out-$i")
    [Console]::Error.WriteLine("err-$i")
}
[Console]::Out.Write(('X' * 100000))
Start-Sleep -Seconds 2
'@
$r = Result streams
$out = Get-Content -Raw $r.stdout
$err = Get-Content -Raw $r.stderr
Assert ($out.Contains('out-99') -and $out.Contains(('X' * 100000))) 'Stdout lost data'
Assert ($err.Contains('err-99') -and -not $out.Contains('err-99')) 'Streams mixed or lost'
$caught = $null
try { Run-Mock nonzero 'exit 37' } catch { $caught = $_ }
Assert ("$caught" -match 'exit code 37') 'Original nonzero exit not preserved'
Assert ((Result nonzero).exitCode -eq 37) 'Exit code metadata incorrect'
$pidPath = Join-Path $root 'descendant.pid'
$escapedPidPath = $pidPath.Replace("'","''")
$caught = $null
$elapsed = [Diagnostics.Stopwatch]::StartNew()
try {
    Run-Mock timeout @"
`$p = Start-Process -FilePath '$($pwsh.Replace("'","''"))' -ArgumentList @('-NoProfile','-Command','Start-Sleep -Seconds 120') -PassThru
Set-Content -LiteralPath '$escapedPidPath' -Value `$p.Id
Start-Sleep -Seconds 120
"@ 8
} catch { $caught = $_ }
Assert ("$caught" -match 'timed out') 'Timeout failure not preserved'
Assert ($elapsed.Elapsed.TotalSeconds -lt 30) 'Timeout cleanup exceeded grace budget'
Assert ((Result timeout).timedOut) 'Timeout metadata incorrect'
Assert (Test-Path $pidPath) 'Mock descendant never launched'
$descendant = [int](Get-Content $pidPath)
Start-Sleep -Seconds 1
Assert (-not (Get-Process -Id $descendant -ErrorAction SilentlyContinue)) 'Descendant survived timeout'
if ($IsWindows) {
    $snapshots = @(Get-Content (Get-ChildItem $root -Filter 'timeout-*.processes.jsonl').FullName | ForEach-Object { $_ | ConvertFrom-Json })
    Assert (@($snapshots.processes.pid) -contains $descendant) 'Snapshot missing descendant'
    Assert (@($snapshots | Where-Object { $_.availableRamBytes -gt 0 -and $_.disks.Count -gt 0 }).Count -gt 0) 'RAM/disk snapshot missing'
    $cmd = Join-Path $root 'mock with spaces.cmd'
    Set-Content $cmd "@echo off`r`necho [%~1]`r`necho cmd-stderr 1>&2`r`nexit /b 23"
    $caught = $null
    try {
        Invoke-MonitoredCommand -Stage cmd -FilePath $cmd -ArgumentList @('argument with spaces') -DiagnosticDirectory $root -TimeoutSeconds 15
    } catch { $caught = $_ }
    Assert ("$caught" -match 'exit code 23') '.cmd exit not preserved'
    Assert ((Get-Content -Raw (Result cmd).stdout).Contains('[argument with spaces]')) '.cmd argument quoting broken'
} else { Write-Host 'SKIP: CIM and .cmd tests require Windows.' }
Write-Host "PASS: monitor mock tests. Diagnostics: $root"
