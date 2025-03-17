#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

API_PRIVATE_KEYS_DIR=$RUNNER_TEMP \
xcrun altool \
  --upload-app \
  --type ios \
  -f build/ios/ipa/*.ipa \
  --apiKey "$APP_STORE_CONNECT_KEYID" \
  --apiIssuer "$APP_STORE_CONNECT_ISSUER" \
  --asc-public-id "$APP_STORE_CONNECT_PUBLIC_ID" \
  --apple-id "$APP_STORE_CONNECT_APP_APPLE_ID" \
  --bundle-id "ci.not.rune.appstore" \
  --bundle-short-version-string "$RUNE_APPSTORE_BUILD_VERSION" \
  --bundle-version "$RUNE_APPSTORE_BUILD_NUMBER" \
  --verbose
