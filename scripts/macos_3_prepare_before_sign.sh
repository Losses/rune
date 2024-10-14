#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

mkdir temp_macos

cp -r build/macos/Build/Products/Release/player.app temp_macos
cp macos/Runner/Release.entitlements temp_macos