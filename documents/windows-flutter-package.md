# Windows Flutter SDK

Use this guide to select the Flutter SDK, change its Nix package, or diagnose SDK initialization. Application build commands are in [Compiling Rune](compiling.md#windows-users).

## Framework compatibility

The project Nix package uses Flutter 3.47.6. Use `nix/platforms/windows/flutter.nix` to prepare it. The launcher applies `flutter-193372.patch` for semantics updates and `flutter-windows-ime-geometry.patch` for Windows composing rectangles to a writable framework copy.

You can also prepare a separate Flutter 3.47.4 installation with `scripts/windows_prepare_flutter_ax_sdk.ps1`. This script applies the same patches.

For a separate SDK, use official Flutter 3.47.4 at revision `9584c6713b324636289d067944a46fd6b49df14b`. Choose a new destination on the development drive:

```powershell
.\scripts\windows_prepare_flutter_ax_sdk.ps1 -SourceSdk D:\SDKs\flutter-3.47.4 -DestinationSdk D:\SDKs\flutter-rune
$env:PATH = "D:\SDKs\flutter-rune\bin;$env:PATH"
flutter pub get
. .\scripts\windows_verify_flutter_framework.ps1
Assert-RuneFlutterFramework -Checkout (Get-Location).Path
```

Replace the example paths with the SDK locations on your machine. `-CheckOnly` checks the source revision, patch hashes and destination without copying. The destination must not already exist.

Run `flutter pub get` after each SDK switch, then run `Assert-RuneFlutterFramework`. The check reads the Flutter package URI in `.dart_tool/package_config.json`. That URI must resolve to the selected patched framework. `windows_build.ps1` performs the same check during its build.

## Change the Nix SDK layout

The launcher sets `FLUTTER_ROOT` to `%LOCALAPPDATA%\Rune\Flutter\<package-output-name>`. When modifying `nix/platforms/windows/flutter-launcher.ps1`, keep the SDK archives and native Dart SDK in immutable Nix outputs.

Copy `bin/cache` into the writable SDK directory, excluding `dart-sdk`, so Flutter can update artifacts and lock files. Copy `packages/flutter_tools` for tool dependency resolution. When framework patches are supplied, copy `packages/flutter` and apply the patches before creating `.ready`. Root metadata and launchers must also be writable.

Use junctions for `bin/cache/dart-sdk` and the remaining SDK directories to reference the selected immutable outputs.

Initialization takes an exclusive file lock with a two-minute timeout and writes `.ready` by atomic rename. If initialization reports an incomplete directory, stop processes using that SDK, rename the incomplete directory and rerun the launcher. Inspect junction targets before removing the renamed directory.

Use Nix package changes to select a new SDK version. The launcher rejects Flutter `upgrade`, `downgrade` and `channel` commands. Artifact and Pub downloads use the writable directories above.

## Upgrade Flutter or a framework patch

1. Update the version, archive and engine identifiers and hashes in `nix/platforms/windows/flutter.nix`. For ARM64, update the native Dart archive hash as well.
2. Check whether the selected Flutter source already implements each patch. Rebase or remove patches and update `windows_prepare_flutter_ax_sdk.ps1`, the version conditions in `flutter.nix`, and the resolved-framework checks together.
3. For the IME patch, check the engine's treatment of composing-rectangle width and height. Apply the extent conversion exactly once. Keep custom `TextInputControl` rectangles in local coordinates and preserve invalid-rectangle values.
4. Resolve packages with the new SDK. Run the application tests and the framework test included in the IME patch:

   ```powershell
   Push-Location D:\SDKs\flutter-rune\packages\flutter
   try {
       & D:\SDKs\flutter-rune\bin\flutter.bat test test/widgets/windows_ime_geometry_compat_test.dart
   } finally {
       Pop-Location
   }
   ```

5. For a Nix package, run `scripts/test_windows_flutter.ps1 -Package <package-output>` on Windows. It checks launcher initialization and reuse, native Dart architecture, cache placement, Windows precache and immutable source metadata.
6. Cold-build Rune and run the [native input, accessibility and DPI checks](application-scaling.md#native-acceptance-cases). Preserve the Fluent Slider's Material semantics when updating the Fluent UI fork. Run `flutter test test/fluent_slider_semantics_test.dart` in that fork.

## Select a build architecture and validate packages

Flutter selects the Windows target from the Dart runtime ABI. Use a native Windows ARM64 runner and ARM64 Dart for an ARM64 build. Select `-Arch arm64` in `scripts/windows_build.ps1`. Supply `-DxcDir` or `RUNE_DXC_DIR` with the ARM64 DirectX Shader Compiler files. Use `-Arch x64` for x64.

Deliver the complete `build/windows/<arch>/runner/Release` directory. With `-BuildInstaller`, the build script also creates `Output/Rune-<arch>-Setup.exe`. The CI workflow in `.github/workflows/build.yml` creates the MSIX with `msix:create` and validates it through `scripts/test_windows_msix.ps1`.

Check the package manifest, executable and native-library architecture, Flutter assets and required runtime DLLs before uploading a package. Include the revision and architecture in its artifact name.
