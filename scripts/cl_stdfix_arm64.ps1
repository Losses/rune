# Transparent cl.exe shim for cc-rs during MSVC ARM64 cross builds.
#
# Why this exists:
#   fdk-aac-sys 0.5.0's FDK_archdef.h recognizes __aarch64__ (GCC/Clang) and
#   _M_ARM (32-bit MSVC ARM) but not _M_ARM64, so MSVC ARM64 compiles fall
#   into its "unknown platform" #warning branch. MSVC 14.51 (VS 18)
#   implements CWG 2518 and makes #warning a fatal error (C1188) unless
#   /std:c++23preview or later is selected.
#   cc-rs compiles the crate's .cpp files in C mode (it only honors CFLAGS_*
#   variables for them) while cl.exe still compiles them as C++ by file
#   extension, so the required flag cannot be injected via CXXFLAGS_*.
#
# Registration: windows_build.ps1 sets CC_aarch64_pc_windows_msvc to this
# script. cc-rs 1.5 passes an existing CC path through as the compiler
# executable and keeps its own MSVC flag generation, so this shim sees the
# full cl command line and can forward it unchanged. For fdk-aac-sys C++
# files it adds /std:c++23preview; everything else is a no-op pass-through.
#
# PowerShell 5.1 compatible (launched via the .ps1 file association).
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$CmdArgs
)

function Find-Cl {
    $cmd = Get-Command cl.exe -ErrorAction SilentlyContinue
    if ($cmd -and $cmd.Source -match '\bin\HostX64\ARM64\') {
        return $cmd.Source
    }
    $roots = @(
        "C:\Program Files\Microsoft Visual Studio",
        "C:\Program Files (x86)\Microsoft Visual Studio"
    )
    foreach ($root in $roots) {
        if (-not (Test-Path $root)) { continue }
        $msvcRoot = Join-Path $root "*\VC\Tools\MSVC"
        $versions = Get-ChildItem -Path $msvcRoot -Directory -ErrorAction SilentlyContinue
        foreach ($v in @($versions | Sort-Object Name -Descending)) {
            $p = Join-Path $v.FullName "bin\HostX64\ARM64\cl.exe"
            if (Test-Path $p) { return $p }
        }
    }
    return $null
}

# Parse the cl-style command line for the compiled source (-c <file>) and the
# object output (-Fo<path> or -Fo <path>).
$src = $null
$fo = $null
for ($i = 0; $i -lt $CmdArgs.Count; $i++) {
    $a = $CmdArgs[$i]
    if ($a -eq "-c") {
        if ($i + 1 -lt $CmdArgs.Count) { $src = $CmdArgs[$i + 1] }
    } elseif ($a -eq "-Fo") {
        if ($i + 1 -lt $CmdArgs.Count) { $fo = $CmdArgs[$i + 1] }
    } elseif ($a.StartsWith("-Fo", [System.StringComparison]::Ordinal)) {
        $fo = $a.Substring(3)
    }
}

$needsStd = $false
if ($src -and $src -match '.(cpp|cc|cxx|c++)$') {
    $cwd = (Get-Location).Path
    if ($cwd -match 'fdk-aac-sys' -or $src -match 'fdk-aac-sys' -or ($fo -and $fo -match 'fdk-aac-sys')) {
        $needsStd = $true
    }
}

$cl = Find-Cl
if (-not $cl) {
    Write-Error "cl_stdfix_arm64.ps1: could not locate cl.exe (HostX64\ARM64) on this machine"
    exit 1
}

if ($needsStd) {
    Write-Warning "cl-stdfix: fdk-aac-sys C++ file $src : adding /std:c++23preview"
    $finalArgs = @("/std:c++23preview") + @($CmdArgs)
} else {
    $finalArgs = @($CmdArgs)
}

& $cl @finalArgs
exit $LASTEXITCODE
