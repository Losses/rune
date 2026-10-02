#!/usr/bin/env sh

set -e

sudo xcode-select -s /Applications/Xcode_27.0.app

cd "$(dirname "$0")"
cd ..

brew install CocoaPods lmdb create-dmg protobuf

# Match the Rinf 8 bindings used by the application.
cargo install rinf_cli --version 8.7.1 --locked

flutter pub global activate protoc_plugin
