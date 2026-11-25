#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

flutter pub get
rinf gen
cd macos
pod update
cd ..
flutter run -d macos
