#!/usr/bin/env sh

set -e

sudo xcode-select -s /Applications/Xcode_16.4.app

cd "$(dirname "$0")"
cd ../../..

brew install flutter rustup protobuf CocoaPods

# Rust
rustup-init -y
. "$HOME/.cargo/env"
rustup default stable
rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin
cargo install rinf_cli

# Flutter
flutter pub global activate protoc_plugin
export PATH="$PATH":"$HOME/.pub-cache/bin"

echo "=== Installed toolchain: ==="
which cargo
cargo --version
which rustc
rustc --version
which rustup
rustup --version
rustup target list --installed

which flutter
flutter --version
