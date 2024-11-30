#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

flutter pub get
rinf message
cd macos
pod update
cd ..
flutter build macos --flavor AppStore --build-number $RUNE_APPSTORE_BUILD_NUMBER --build-name $RUNE_APPSTORE_BUILD_VERSION --release
chmod -R +x build/macos/Build/Products/Release-AppStore/Rune.app
xattr -cr build/macos/Build/Products/Release-AppStore/Rune.app
