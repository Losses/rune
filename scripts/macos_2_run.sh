#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

flutter pub get
rinf message
cd macos
pod update
cd ..
flutter run
