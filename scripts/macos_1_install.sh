#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

brew install --cask flutter

brew install CocoaPods lmdb create-dmg rust rustup protobuf

rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

flutter pub global activate protoc_plugin
export PATH="$PATH":"$HOME/.pub-cache/bin"

cargo install 'flutter_rust_bridge_codegen' rinf protoc-gen-prost

echo "DEBUG: ----------------------------"

which cargo
which rustc
which rustup
rustup target list --installed