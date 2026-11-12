#!/usr/bin/env sh

# Builds an IPA without code signing, for people who sign it themselves.
# Usage: 2_build_unsigned_ipa.sh [OUTPUT], OUTPUT defaulting to build/ios/ipa/Rune-unsigned.ipa.

set -e

cd "$(dirname "$0")"
cd ../../..

OUTPUT="${1:-build/ios/ipa/Rune-unsigned.ipa}"

flutter pub get
rinf gen
cd ios
pod update
cd ..

flutter build ios --release --no-codesign

# An IPA is the app bundle inside a Payload folder, zipped.
STAGE="$(mktemp -d)"
mkdir "$STAGE/Payload"
cp -R build/ios/iphoneos/*.app "$STAGE/Payload/"
mkdir -p "$(dirname "$OUTPUT")"
OUTPUT="$(cd "$(dirname "$OUTPUT")" && pwd)/$(basename "$OUTPUT")"
rm -f "$OUTPUT"
(cd "$STAGE" && zip -qry "$OUTPUT" Payload)
rm -rf "$STAGE"
echo "Built $OUTPUT"
