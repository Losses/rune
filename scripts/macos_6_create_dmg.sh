#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

# if $REF_NAME exists
if [ -n "$REF_NAME" ]; then
  REF_NAME="-$REF_NAME"
fi

create-dmg \
  --volname "Rune$REF_NAME-macOS" \
  --window-pos 200 120 \
  --window-size 800 450 \
  --icon-size 100 \
  --app-drop-link 600 185 \
  "Rune$REF_NAME-macOS.dmg" \
  Rune.app