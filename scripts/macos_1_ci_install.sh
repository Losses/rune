#!/usr/bin/env sh

set -e

sudo xcode-select -s /Applications/Xcode_16.4.app

cd "$(dirname "$0")"
cd ..

brew install CocoaPods lmdb create-dmg

cargo install rinf_cli

flutter pub global activate protoc_plugin
