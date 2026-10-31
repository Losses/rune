# Execute on the interactive desktop only for the dedicated Rune test PID.
# Coordinates are physical client pixels, such as Flutter global logical
# coordinates multiplied by the system DPR. Restores the previous cursor.
param([Parameter(Mandatory)][int]$ProcessId, [Parameter(Mandatory)][int]$ClientX, [Parameter(Mandatory)][int]$ClientY, [switch]$KeepFocus, [switch]$KeepCursor)
$ErrorActionPreference='Stop'
if (-not ('RuneClick' -as [type])) { Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class RuneClick {
 [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X,Y; }
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
 [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll")] public static extern IntPtr GetThreadDpiAwarenessContext();
 [DllImport("user32.dll")] public static extern int GetAwarenessFromDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref POINT p);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out RECT r);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint dx,uint dy,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr h,bool altTab);
}
'@
}
[RuneClick]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null
if ([RuneClick]::GetAwarenessFromDpiAwarenessContext([RuneClick]::GetThreadDpiAwarenessContext()) -ne 2) { throw 'Per-monitor DPI awareness unavailable.' }
$p=Get-Process -Id $ProcessId; $h=$p.MainWindowHandle
if ($h -eq [IntPtr]::Zero) { throw 'No test window.' }
$rect=New-Object RuneClick+RECT
if (-not [RuneClick]::GetClientRect($h,[ref]$rect) -or $ClientX -lt 0 -or $ClientY -lt 0 -or $ClientX -ge $rect.Right -or $ClientY -ge $rect.Bottom) { throw 'Point outside target client.' }
if (-not $KeepFocus) { [RuneClick]::SwitchToThisWindow($h,$true); Start-Sleep -Milliseconds 250 }
if ([RuneClick]::GetForegroundWindow() -ne $h) { throw 'Test window could not be focused.' }
$point=New-Object RuneClick+POINT; $point.X=$ClientX; $point.Y=$ClientY
if (-not [RuneClick]::ClientToScreen($h,[ref]$point)) { throw 'ClientToScreen failed.' }
[uint32]$pidAtPoint=0
$at=[RuneClick]::WindowFromPoint($point); [RuneClick]::GetWindowThreadProcessId($at,[ref]$pidAtPoint) | Out-Null
if ($pidAtPoint -ne $ProcessId) { throw 'Click refused: point belongs to another process.' }
$previous=New-Object RuneClick+POINT; [RuneClick]::GetCursorPos([ref]$previous) | Out-Null
try {
 [RuneClick]::SetCursorPos($point.X,$point.Y) | Out-Null
 [RuneClick]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 200
 [RuneClick]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
 Start-Sleep -Milliseconds 300
} finally { if (-not $KeepCursor) { [RuneClick]::SetCursorPos($previous.X,$previous.Y) | Out-Null } }
