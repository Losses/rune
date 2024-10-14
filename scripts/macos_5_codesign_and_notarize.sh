#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

echo "Codesign: ----------------------------"

# DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY must use Developer ID Application certificate, or app cannot be notarized
/usr/bin/codesign --deep --force -s "$DEVELOPER_ID_APPLICATION_SIGNING_IDENTITY" --options runtime player.app -v

echo "Notarize: ----------------------------"

/usr/bin/ditto -c -k --keepParent "player.app" "player.zip"

# APPLE_PASSWORD must use app-specific password
xcrun notarytool submit "player.zip" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD" --wait

xcrun stapler staple "player.app"
