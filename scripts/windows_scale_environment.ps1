# Execute inside the logged-in interactive Windows session; writes only the
# supplied report file. Process DPI awareness applies only to this process.
param([Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class RuneDisplayInfo {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
  [DllImport("user32.dll")] public static extern IntPtr MonitorFromPoint(POINT p, uint flags);
  [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr m, int kind, out uint x, out uint y);
  [DllImport("shcore.dll")] public static extern int SetProcessDpiAwareness(int awareness);
}
'@
[RuneDisplayInfo]::SetProcessDpiAwareness(2) | Out-Null
$lines = @("Session=$([Diagnostics.Process]::GetCurrentProcess().SessionId)")
foreach ($screen in [System.Windows.Forms.Screen]::AllScreens) {
  $p = New-Object RuneDisplayInfo+POINT
  $p.X = $screen.Bounds.Left + [int]($screen.Bounds.Width / 2)
  $p.Y = $screen.Bounds.Top + [int]($screen.Bounds.Height / 2)
  $monitor = [RuneDisplayInfo]::MonitorFromPoint($p, 2)
  [uint32]$x = 0; [uint32]$y = 0
  $result = [RuneDisplayInfo]::GetDpiForMonitor($monitor, 0, [ref]$x, [ref]$y)
  $lines += "Display=$($screen.DeviceName); Bounds=$($screen.Bounds); Primary=$($screen.Primary); DpiX=$x; DpiY=$y; DpiResult=$result"
}
foreach ($lang in Get-WinUserLanguageList) {
  $lines += "Language=$($lang.LanguageTag); InputMethods=$($lang.InputMethodTips -join ',')"
}
$lines | Set-Content -Encoding UTF8 -LiteralPath $OutputPath
