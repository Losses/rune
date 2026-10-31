# Check the framework actually resolved by pub, rather than the Flutter binary
# alone: package_config.json can still point to a previous SDK after a switch.
function Assert-RuneFlutterFramework {
    param([Parameter(Mandatory)][string]$Checkout)
    $configPath = Join-Path $Checkout '.dart_tool\package_config.json'
    $helpText = 'Use a supported private Flutter SDK with both Rune patches. For Flutter 3.47.4 run: powershell -File scripts\windows_prepare_flutter_ax_sdk.ps1 -SourceSdk C:\path\flutter -DestinationSdk C:\path\flutter-rune; put flutter-rune\bin on PATH, then run flutter pub get. Nix Windows Flutter 3.47.6 provides its own patched private facade.'
    if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) { throw "Flutter package configuration missing. $helpText" }
    $config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
    $flutter = @($config.packages | Where-Object { $_.name -eq 'flutter' })
    if ($flutter.Count -ne 1) { throw "Expected one Flutter package in package_config.json. $helpText" }
    try {
        $base = [Uri]::new((Resolve-Path -LiteralPath $configPath).Path)
        $uri = [Uri]::new($base, [string]$flutter[0].rootUri)
        if (-not $uri.IsFile) { throw 'Flutter package URI is not a local file.' }
        $package = $uri.LocalPath
        $semantics = Get-Content -LiteralPath (Join-Path $package 'lib\src\semantics\semantics.dart') -Raw
        $inputSource = Get-Content -LiteralPath (Join-Path $package 'lib\src\services\text_input.dart') -Raw
    } catch { throw "Unable to inspect resolved Flutter package: $_. $helpText" }
    if (-not $semantics.Contains('_isSkippedInHitTestTree') -or -not $inputSource.Contains('_windowsEngineComposingRect')) {
        throw "Resolved Flutter package lacks the required accessibility or Windows IME compatibility patch: $package. $helpText"
    }
    Write-Host "Verified patched Flutter framework: $package"
}
