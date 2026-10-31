# Run on a Windows desktop from the repository root. No system settings change.
param([switch]$RunWidgetTests)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'Run this script on Windows.' }
Write-Host "OS: $([System.Environment]::OSVersion.VersionString)"
Write-Host "Architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
flutter --version
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Screen]::AllScreens | ForEach-Object {
  Write-Host "Display $($_.DeviceName): bounds=$($_.Bounds), primary=$($_.Primary)"
}
Write-Host 'Record each display scale in Windows Settings > System > Display. Bounds may be DPI-virtualized.'
Write-Host 'Follow documents/application-scaling.md#native-acceptance-cases for physical IME, drag/drop and monitor-transition checks.'
if ($RunWidgetTests) {
  flutter test --no-pub lib/test/providers/app_viewport_test.dart lib/test/widgets/scaled_flyout_test.dart lib/test/widgets/app_drop_target_test.dart lib/test/widgets/fluent_overlay_geometry_test.dart
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  Write-Host 'These widget tests do not validate native Windows input or displays.'
}
