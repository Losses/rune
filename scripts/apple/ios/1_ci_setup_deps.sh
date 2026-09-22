#!/usr/bin/env sh

set -e

sudo xcode-select -s /Applications/Xcode_16.4.app

cd "$(dirname "$0")"
cd ../../..

brew install CocoaPods protobuf

# Rust
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  . "$HOME/.cargo/env"
fi
rustup default stable || true
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim || true
cargo install rinf_cli || true

# Flutter
if ! command -v flutter >/dev/null 2>&1; then
  brew install flutter
fi
flutter pub global activate protoc_plugin || true
export PATH="$PATH":"$HOME/.pub-cache/bin"

echo "=== Installed toolchain: ==="
which cargo || true
cargo --version || true
which rustc || true
rustc --version || true
which rustup || true
rustup --version || true
rustup target list --installed || true

which flutter || true
flutter --version || true

