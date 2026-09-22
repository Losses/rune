# Rune Player Development Guidelines

## Dev Environment (Nix)
- Enter with `nix develop`; Android cross-builds need `setup_android_env` first.
- Rust and Flutter versions are pinned in `default.devshell.nix` (`rustVersion`, `flutterPkg`). Bump those two lines to upgrade; `nix flake update` alone will not move them.
- Do NOT add the `rustup` package to the devshell. rinf's cargokit always builds through `rustup` (checking `$HOME/.cargo/bin` before PATH); the devshell ships a shim `rustup` that forwards to the Nix toolchain. If `$HOME/.cargo/bin/rustup` exists it will silently win — delete it.

## Build/Test/Lint Commands
- Build: `flutter build` (platform-specific flags can be added)
- Run: `flutter run` or `./scripts/macos_2_run.sh` (macOS)
- Lint: `just lint` (runs Rust and Flutter linting together)
- Rust Lint: `cargo fmt -- --check && cargo clippy -- -D warnings`
- Flutter Lint: `flutter analyze .`
- Rust Tests: `cargo test` (run a single test: `cargo test test_name`)
- Rust Bench: `cargo bench` (in analysis dir)
- Flutter Tests: `flutter test` (for a single test: `flutter test test/widget_test.dart`)

## Code Style Guidelines
- Dart: Follow Flutter lints & prefer relative imports
- Rust: Follow Rust 2021 edition and clippy lints
- Error handling: Use Anyhow for Rust, proper try/catch in Dart
- Naming: Follow language idioms (snake_case for Rust, camelCase for Dart)
- Types: Use strong typing, avoid `any` in Dart, prefer anyhow::Result<T> in Rust
- Organization: Follow component-based structure, message-passing architecture
- PRs: Keep changes focused, ensure lints pass for all platforms
- Documentation: Document public APIs with doc comments