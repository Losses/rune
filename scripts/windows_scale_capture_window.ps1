# Invoke in the target interactive desktop. Captures only the specified process.
param([Parameter(Mandatory)][int]$ProcessId, [Parameter(Mandatory)][string]$OutputPath)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class RuneScaleCapture {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll", SetLastError=true)] public static extern bool SetProcessDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern IntPtr GetThreadDpiAwarenessContext();
  [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr context);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT rect);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT rect);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT point);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
}
'@
# -4 is DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2. This affects only
# this helper process and must run before GDI/WinForms create any windows.
$setDpi = [RuneScaleCapture]::SetProcessDpiAwarenessContext([IntPtr](-4))
$awareness = [RuneScaleCapture]::GetAwarenessFromDpiAwarenessContext([RuneScaleCapture]::GetThreadDpiAwarenessContext())
if ($awareness -ne 2) { throw 'Could not establish per-monitor DPI awareness for the screenshot helper.' }
Add-Type -AssemblyName System.Drawing
$process = Get-Process -Id $ProcessId
$handle = $process.MainWindowHandle
if ($handle -eq [IntPtr]::Zero) { throw 'The selected process has no main window.' }
$rect = New-Object RuneScaleCapture+RECT
$client = New-Object RuneScaleCapture+RECT
$origin = New-Object RuneScaleCapture+POINT
if (-not [RuneScaleCapture]::GetWindowRect($handle, [ref]$rect)) { throw 'GetWindowRect failed.' }
if (-not [RuneScaleCapture]::GetClientRect($handle, [ref]$client)) { throw 'GetClientRect failed.' }
if (-not [RuneScaleCapture]::ClientToScreen($handle, [ref]$origin)) { throw 'ClientToScreen failed.' }
$width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) { throw 'The selected window has no visible size.' }
$bitmap = New-Object System.Drawing.Bitmap($width, $height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
try {
  $dc = $graphics.GetHdc()
  try { $ok = [RuneScaleCapture]::PrintWindow($handle, $dc, 2) }
  finally { $graphics.ReleaseHdc($dc) }
  if (-not $ok) { throw 'PrintWindow failed.' }
  $bitmap.Save($OutputPath, [System.Drawing.Imaging.ImageFormat]::Png)
  $nonBlack = 0; $samples = 0
  for ($y = 0; $y -lt $height; $y += [Math]::Max(1, [int]($height / 32))) {
    for ($x = 0; $x -lt $width; $x += [Math]::Max(1, [int]($width / 32))) {
      $color = $bitmap.GetPixel($x, $y); $samples++
      if ($color.R -gt 3 -or $color.G -gt 3 -or $color.B -gt 3) { $nonBlack++ }
    }
  }
  $info = [ordered]@{
    processId = $ProcessId; sessionId = $process.SessionId
    dpiAwareness = $awareness; perMonitorV2Set = $setDpi
    windowDpi = [RuneScaleCapture]::GetDpiForWindow($handle)
    windowRect = @($rect.Left, $rect.Top, $rect.Right, $rect.Bottom)
    clientRect = @($origin.X, $origin.Y, ($origin.X + $client.Right), ($origin.Y + $client.Bottom))
    imageSize = @($width, $height); printWindow = $ok
    nonBlackSamples = $nonBlack; totalSamples = $samples
  }
  $info | ConvertTo-Json | Set-Content -Encoding UTF8 -LiteralPath ($OutputPath + '.json')
  if ($nonBlack -eq 0) { throw 'PrintWindow returned an entirely black sampled image; inspect capture capability.' }
  Write-Output 'Window capture produced nonblack samples; inspect image for correct GPU-rendered content.'
} finally { $graphics.Dispose(); $bitmap.Dispose() }
