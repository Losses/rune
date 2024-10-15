#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

create-dmg \
  --volname "Rune-$REF_NAME-$SHA-macOS" \
  --window-pos 200 120 \
  --window-size 800 450 \
  --icon-size 100 \
  --app-drop-link 600 185 \
  "Rune-$REF_NAME-$SHA-macOS.dmg" \
  Rune.app