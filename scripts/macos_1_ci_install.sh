#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

brew install CocoaPods lmdb create-dmg

cargo install rinf_cli

flutter pub global activate protoc_plugin