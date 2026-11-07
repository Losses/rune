# Run after activating msvc.env. Use -CompileOnly for the ARM64 target on x64.
param([Parameter(Mandatory)][string]$WorkDirectory, [switch]$CompileOnly)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force $WorkDirectory | Out-Null
$WorkDirectory = (Resolve-Path $WorkDirectory).Path
$source = Join-Path $WorkDirectory 'msvc-smoke.cpp'
@'
#include <Windows.h>
#include <algorithm>
#include <vector>

#if defined(_M_X64)
#pragma comment(linker, "/include:__std_search_1")
#pragma comment(linker, "/include:__std_remove_8")
#pragma comment(linker, "/include:__std_replace_8")
#endif

int main() {
    std::vector<long long> values{1, 2, 3, 2};
    std::replace(values.begin(), values.end(), 2LL, 4LL);
    values.erase(std::remove(values.begin(), values.end(), 4LL), values.end());
    return values.size() == 2 && GetCurrentProcessId() ? 0 : 1;
}
'@ | Set-Content -Encoding ASCII $source
function Invoke-SmokeProcess([string]$Executable, [string]$Arguments, [int]$Seconds) {
    $process = New-Object Diagnostics.Process
    $process.StartInfo.FileName = $Executable
    $process.StartInfo.Arguments = $Arguments
    $process.StartInfo.WorkingDirectory = $WorkDirectory
    $process.StartInfo.UseShellExecute = $false
    [void]$process.Start()
    try {
        if (-not $process.WaitForExit($Seconds * 1000)) {
            & taskkill.exe /PID $process.Id /T /F | Out-Null
            throw 'MSVC smoke process timed out'
        }
        if ($process.ExitCode -ne 0) { throw "MSVC smoke process failed ($($process.ExitCode))" }
    } finally { $process.Dispose() }
}
$compiler = (Get-Command cl.exe -ErrorAction Stop).Source
Invoke-SmokeProcess $compiler '/nologo /EHsc /MT /O2 msvc-smoke.cpp /Fomsvc-smoke.obj /Femsvc-smoke.exe' 120
if (-not $CompileOnly) { Invoke-SmokeProcess (Join-Path $WorkDirectory 'msvc-smoke.exe') '' 30 }
Write-Host 'MSVC smoke test passed'
