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

   **For non-Nix users**:

   - Use the provided build script that automatically sets up the environment and builds the APK:

     ```bash
     ./android/build.sh
     ```

### Windows Users

1. **Configure Development Environment**:

   - **Flutter SDK**: [Installation Guide](https://docs.flutter.dev/get-started/install)
   - **Rust Toolchain**: [Installation Guide](https://www.rust-lang.org/tools/install)

   Verify your setup with:

   ```bash
   rustc --version
   flutter doctor
   ```

2. **Compile Rune**:

   ```powershell
   flutter pub get
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
