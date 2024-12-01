#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

rm -rf temp_macos
mkdir temp_macos

ditto build/macos/Build/Products/Release/Rune.app temp_macos/Rune.app
cp macos/Runner/Release.entitlements temp_macos
