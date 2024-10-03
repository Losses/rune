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
     flutter pub run rinf message
     flutter build linux --release
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
    flutter pub run rinf message
    flutter build windows --release
    ```

### Protobuf Messages

If you’ve cloned the project or modified `.proto` files in the `./messages` directory, run:

```bash
flutter pub run rinf message
```

### Running the App

Build and run the app with:

```bash
flutter run
```

For detailed integration instructions, refer to Rinf's [documentation](https://rinf.cunarist.com).

## Tips for Compiling on macOS

Rune currently does not support macOS, but here are some tips for attempting compilation:

- Ensure you have the Flutter SDK and Rust toolchain installed.
- For [CocoaPods](https://cocoapods.org/), avoid using the default macOS Ruby version. Use Homebrew instead:

  ```bash
  brew install cocoapods
  ```
