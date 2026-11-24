# Rune Player Development Guidelines

## Build/Test/Lint Commands
- Build: `flutter build` (platform-specific flags can be added)
- Run: `flutter run` or `./scripts/macos_2_run.sh` (macOS)
- Rust Lint: `cargo fmt -- --check && cargo clippy -- -D warnings`
- Flutter Lint: `flutter analyze .`
- Rust Tests: `cargo test` (run a single test: `cargo test test_name`)
- Rust Bench: `cargo bench` (in analysis dir)
- Flutter Tests: `flutter test lib/test` (for a single test: `flutter test lib/test/widgets/reveal_update_test.dart`)

## Code Style Guidelines
- Dart: Follow Flutter lints & prefer relative imports
- Rust: Follow Rust 2021 edition and clippy lints
- Error handling: Use Anyhow for Rust, proper try/catch in Dart
- Naming: Follow language idioms (snake_case for Rust, camelCase for Dart)
- Types: Use strong typing, avoid `any` in Dart, prefer anyhow::Result<T> in Rust
- Organization: Rust workspace crates live at the repository root; Flutter application code and tests live under `lib/` and `lib/test/`. Follow component-based structure and message-passing architecture.
- UI dependencies: Maintain Fluent UI changes in `Losses/fluent_ui`, pin its Git commit in `pubspec.yaml`, and update `pubspec.lock` and the Nix Git hash. Do not copy the dependency into this repository.
- PRs: Keep changes focused, ensure lints pass for all platforms
- Documentation: Document public APIs with doc comments