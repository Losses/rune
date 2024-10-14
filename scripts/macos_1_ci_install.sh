#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

brew install CocoaPods lmdb create-dmg

cargo install 'flutter_rust_bridge_codegen' rinf protoc-gen-prost

flutter pub global activate protoc_plugin