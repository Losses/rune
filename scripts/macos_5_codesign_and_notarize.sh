#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

echo "Codesign: ----------------------------"

# DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY must use Developer ID Application certificate, or app cannot be notarized
# /usr/bin/codesign --deep --force -s "$DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY" --options runtime player.app -v

/usr/bin/codesign \
  --deep \
  --force \
  -s "$DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY" \
  --options runtime \
  -v \
  Rune.app

/usr/bin/codesign \
  --force \
  -s "$DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY" \
  --entitlements Release.entitlements \
  --options runtime \
  -v \
  Rune.app

echo "Notarize: ----------------------------"

/usr/bin/ditto -c -k --keepParent "Rune.app" "Rune.zip"

# APPLE_PASSWORD must use app-specific password
xcrun notarytool submit "Rune.zip" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" --wait

xcrun stapler staple "Rune.app"
