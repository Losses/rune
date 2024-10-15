#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

echo "----------------------------"

otool -L Rune.app/Contents/MacOS/Rune | grep lmdb

echo "----------------------------"

install_name_tool -change /opt/homebrew/opt/lmdb/lib/liblmdb.dylib @executable_path/../Frameworks/liblmdb.dylib Rune.app/Contents/MacOS/Rune

echo "----------------------------"

otool -L Rune.app/Contents/MacOS/Rune | grep lmdb