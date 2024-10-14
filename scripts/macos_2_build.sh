#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

flutter pub get
rinf message
cd macos
pod update
cd ..
flutter build macos --release --verbose
chmod -R +x build/macos/Build/Products/Release/player.app
xattr -cr build/macos/Build/Products/Release/player.app
