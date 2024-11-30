#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..
cd temp_macos

echo "Codesign: ----------------------------"

/usr/bin/codesign \
  --deep \
  --force \
  -s "$APPLE_DISTRIBUTION_SIGNING_IDENTITY" \
  --entitlements Release.entitlements \
  --options runtime \
  Rune.app \
  -v

echo "Package: ----------------------------"

xcrun productbuild \
  --sign "$MAC_DEVELOPER_INSTALLER_SIGNING_IDENTITY" \
  --component Rune.app \
  /Applications \
  Rune.pkg

echo "Upload to App Store Connect: ----------------------------"

API_PRIVATE_KEYS_DIR=$RUNNER_TEMP \
xcrun altool \
  --upload-package Rune.pkg \
  --type osx \
  --apiKey "$APP_STORE_CONNECT_KEYID" \
  --apiIssuer "$APP_STORE_CONNECT_ISSUER" \
  --asc-public-id "$APP_STORE_CONNECT_PUBLIC_ID" \
  --apple-id "$APP_STORE_CONNECT_APP_APPLE_ID" \
  --bundle-id "ci.not.rune.appstore" \
  --bundle-short-version-string "$RUNE_APPSTORE_BUILD_VERSION" \
  --bundle-version "$RUNE_APPSTORE_BUILD_NUMBER" \
  --verbose
