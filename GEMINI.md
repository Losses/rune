# Rune Player Development Guidelines

## Build/Test/Lint Commands
- Build: `flutter build` (platform-specific flags can be added)
- Run: `flutter run` or `./scripts/macos_2_run.sh` (macOS)
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