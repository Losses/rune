#!/usr/bin/env zsh

set -e

cd "$(dirname "$0")"
cd ..

brew install --cask flutter

brew install cocoapods 
brew install lmdb create-dmg rust rustup protobuf

rustup target add x86_64-apple-darwin
rustup target add aarch64-apple-darwin

flutter pub global activate protoc_plugin
export PATH="$PATH":"$HOME/.pub-cache/bin"

cargo install 'flutter_rust_bridge_codegen' rinf protoc-gen-prost

echo "DEBUG: ----------------------------"

where cargo
where rustc
where rustup
rustup target list --installed