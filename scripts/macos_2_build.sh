#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

flutter pub get
rinf gen
cd macos
pod update
cd ..
flutter build macos --release
chmod -R +x build/macos/Build/Products/Release/Rune.app
xattr -cr build/macos/Build/Products/Release/Rune.app
