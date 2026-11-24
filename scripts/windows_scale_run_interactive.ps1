# Run on Windows via SSH. Starts exactly the caller-supplied executable in the
# already logged-in user's desktop; does not change DPI or user settings.
param([Parameter(Mandatory)][string]$Executable, [string]$Arguments = '')
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) { throw 'Executable not found.' }
$name = 'RuneScaleProbe-' + [Guid]::NewGuid().ToString('N')
$action = New-ScheduledTaskAction -Execute $Executable -Argument $Arguments
$principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited
try {
  Register-ScheduledTask -TaskName $name -Action $action -Principal $principal | Out-Null
  Start-ScheduledTask -TaskName $name
  # Wait until the scheduler has started the action before removing the task.
  for ($i = 0; $i -lt 50; $i++) {
    $info = Get-ScheduledTaskInfo -TaskName $name
    if ($info.LastRunTime.Year -gt 2000) { break }
    Start-Sleep -Milliseconds 100
  }
  if ($info.LastRunTime.Year -le 2000) { throw 'Interactive task did not start.' }
} finally {
  Unregister-ScheduledTask -TaskName $name -Confirm:$false -ErrorAction SilentlyContinue
}
