## Setting up Environment

### Linux Users

1. **Set Up Development Environment**:

   - Install [Nix](https://nixos.org/download/#download-nix) and [Flakes](https://nixos.wiki/wiki/Flakes).
   - Clone the repository:

     ```bash
     git clone https://github.com/losses/rune.git
     cd rune
     ```

   - Set up the environment:

     ```bash
     nix develop
     ```

2. **Compile Rune**:

   - Compile for Linux:

     ```bash
     rinf gen
     flutter build linux --release
     ```

3. **Android Build Setup**:

   **For Nix users**:

   - Enter the Nix shell:

     ```bash
     nix develop .
     ```

   - Set up Android environment:

     ```bash
     setup_android_env
     ```

   - Build Android APK:

     ```bash
     flutter build apk --release
     ```

     > **Note**: The x86 build will not work with this command after setting up the Android environment.

   - To switch back from Android environment:

     ```bash
     teardown_android_env
     ```

   **On-device debugging**:

   - The settings Laboratory (unlock via the mysterious button on the About
     page) contains a "Filesystem Self-Test" card that runs layered Dart →
     Rust → SAF diagnostics against any picked directory, including TF cards.
   - `scripts/android/saf_selftest.sh` collects logcat around a manual run;
     `scripts/android/emulator_selftest.sh` drives the whole flow on the
     `selftest` emulator AVD (see the script header for setup).
   - For manual `cargo check --target aarch64-linux-android`, export the NDK
     compilers first: `CC_aarch64_linux_android=aarch64-linux-android21-clang`
     `CXX_aarch64_linux_android=aarch64-linux-android21-clang++`
     `AR_aarch64_linux_android=llvm-ar` (after `setup_android_env`).

   **For non-Nix users**:

   - Use the provided build script that automatically sets up the environment and builds the APK:

     ```bash
     ./android/build.sh
     ```

### Windows Users

1. **Configure Development Environment**:

   - **Flutter SDK**: follow the [Windows SDK preparation and compatibility requirements](windows-flutter-package.md#framework-compatibility). An arbitrary unpatched installation does not satisfy the current application build requirements.
   - **Rust Toolchain**: [Installation Guide](https://www.rust-lang.org/tools/install)

   Verify your setup with:

   ```bash
   rustc --version
   flutter doctor
   ```

2. **Compile Rune**:

   ```powershell
   flutter pub get
   . .\scripts\windows_verify_flutter_framework.ps1
   Assert-RuneFlutterFramework -Checkout (Get-Location).Path
   rinf gen
   flutter build windows --release
   ```

### Protobuf Messages

If you've cloned the project or modified `.proto` files in the `./messages` directory, run:

```bash
rinf gen
```

### Running the App

Build and run the app with:

```bash
flutter run
```

For detailed integration instructions, refer to Rinf's [documentation](https://rinf.cunarist.com).

## macOS

> This chapter provides instructions for building Rune on macOS for **development**. If you want to fork Rune and build your own version for production, you need to set up your own code signing, provisioning profiles, etc., which is not covered in this chapter.

### Prerequisites

- Xcode
- [Homebrew](https://brew.sh)

### Steps

1. Clone the repository:

```bash
git clone https://github.com/losses/rune.git
cd rune
```

2. Install all development dependencies:

```sh
./scripts/macos_1_install.sh
```

> If you're an employee of _Inkwire Tech_, make sure you have an Apple Account in _Inkwire Tech_'s Developer Program logged in on your Xcode, and skip to Step #6. Ask @laosb if you can't make it work.

3. Open the project in Xcode:

```sh
open ./macos/Runner.xcworkspace
```

4. In Xcode, select the `Runner` project in the project navigator, then select the `Runner` target.
5. In the _Signing & Capabilities_ tab:
6. Uncheck _Automatically manage signing_.
7. Select _None_ for _Provisioning Profile_.
8. Select _None_ for _Team_.
9. Select _Sign to Run Locally_ for _Signing Certificate_.
10. Build / run the project:

```sh
./scripts/macos_2_build.sh
# or
./scripts/macos_2_run.sh
```

We use the signing configuration in our production GitHub Actions workflow, so please don't commit and push any changes to the signing configuration.

### Development storage and test isolation

When using a separate development volume, place the checkout and SDKs there and set the following paths before installing dependencies or building:

- Set `PUB_CACHE` for Pub.
- Set `CARGO_HOME` and `CARGO_TARGET_DIR` for Cargo.
- Set `GEM_HOME`, `GEM_PATH` and `GEM_SPEC_CACHE` for Ruby.
- Set `CP_HOME_DIR` and `CP_CACHE_DIR` for CocoaPods.
- Set `BUNDLE_PATH`, `BUNDLE_USER_HOME` and `BUNDLE_USER_CACHE` for Bundler.
- Set `TMPDIR`, `CLANG_MODULE_CACHE_PATH` and `SWIFT_MODULECACHE_PATH` for compiler temporary files and module caches.

To use `scripts/macos_scale_run.sh`, set `RUNE_SCALE_VOLUME` to the mounted development volume and `RUNE_SCALE_TOOLS` to an existing tools directory on that volume. Install Flutter under that directory as `flutter` and make CMake available on `PATH`. The wrapper checks storage before running the supplied command.

Create those directories on the selected volume and check their resolved paths with `df -h`. Keep `HOME` at its normal location. Inspect the Xcode build log for derived-data and module-cache paths, and the Cargo log for the target directory. macOS app containers and system caches still require free space on the system volume.

Use a separate checkout and test library for native input tests. To isolate macOS application files, give the test build a separate bundle identifier and container. Keep this signing configuration local to the test checkout. Prepare silent settings and run the checks in [Application scaling](application-scaling.md#native-acceptance-cases).
