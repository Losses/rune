#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ../../..

# Adjust build settings to use correct bundle ID, provisioning profile, entitlements, and signing identity.
cp ios/Runner.xcodeproj/project.pbxproj ios/Runner.xcodeproj/project.pbxproj.backup
sed -i '' 's/PRODUCT_BUNDLE_IDENTIFIER = ci.not.rune;/PRODUCT_BUNDLE_IDENTIFIER = ci.not.rune.appstore;/g' ios/Runner.xcodeproj/project.pbxproj
sed -i '' 's/<string>LG57TUQ726.ci.not.rune<\/string>/<string>LG57TUQ726.ci.not.rune.appstore<\/string>/g' ios/Runner/Runner.entitlements
sed -i '' 's/"PROVISIONING_PROFILE_SPECIFIER\[sdk=iphoneos\*]" = "Rune";/"PROVISIONING_PROFILE_SPECIFIER[sdk=iphoneos*]" = "Rune iOS App Store";/g' ios/Runner.xcodeproj/project.pbxproj

flutter pub get
rinf gen
cd ios
pod update
cd ..

flutter build ipa \
  --build-number $RUNE_APPSTORE_BUILD_NUMBER \
  --build-name $RUNE_APPSTORE_BUILD_VERSION \
  --export-options-plist ios/ExportOptions-AppStore.plist \
  --release
