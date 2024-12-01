#!/bin/sh

set -e

if [ "$1" != "release" ] && [ "$1" != "debug" ]; then
    echo "Usage: $0 <release|debug>"
    exit 1
fi

cd "$(dirname "$0")"

rm -rf lib
mkdir -p lib
swift package clean

swift build --arch arm64 --configuration "$1" -Xswiftc -static -Xswiftc -import-objc-header -Xswiftc ./Sources/apple-bridge-library/bridging-header.h
swift build --arch x86_64 --configuration "$1" -Xswiftc -static -Xswiftc -import-objc-header -Xswiftc ./Sources/apple-bridge-library/bridging-header.h

lipo -create \
".build/arm64-apple-macosx/$1/libapple-bridge-library.a" \
".build/x86_64-apple-macosx/$1/libapple-bridge-library.a" \
-output "lib/libapple-bridge-library.a"