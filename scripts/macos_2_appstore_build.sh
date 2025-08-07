#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

# Create backup of project.pbxproj
cp macos/Runner.xcodeproj/project.pbxproj macos/Runner.xcodeproj/project.pbxproj.backup

# Patch macos/Runner.xcodeproj/project.pbxproj and macos/Runner/Release.entitlements to replace bundle ID and provisioning profile
sed -i '' 's/PRODUCT_BUNDLE_IDENTIFIER = ci.not.rune;/PRODUCT_BUNDLE_IDENTIFIER = ci.not.rune.appstore;/g' macos/Runner.xcodeproj/project.pbxproj
sed -i '' 's/"PROVISIONING_PROFILE_SPECIFIER\[sdk=macosx\*]" = "Rune Notarized";/"PROVISIONING_PROFILE_SPECIFIER[sdk=macosx*]" = "Rune App Store";/g' macos/Runner.xcodeproj/project.pbxproj
sed -i '' 's/"CODE_SIGN_IDENTITY\[sdk=macosx\*]" = "Developer ID Application";/"CODE_SIGN_IDENTITY[sdk=macosx*]" = "3rd Party Mac Developer Application";/g' macos/Runner.xcodeproj/project.pbxproj
sed -i '' 's/<string>LG57TUQ726.ci.not.rune<\/string>/<string>LG57TUQ726.ci.not.rune.appstore<\/string>/g' macos/Runner/Release.entitlements

flutter pub get
rinf gen
cd macos
pod update
cd ..
flutter build macos --build-number $RUNE_APPSTORE_BUILD_NUMBER --build-name $RUNE_APPSTORE_BUILD_VERSION --release
chmod -R +x build/macos/Build/Products/Release/Rune.app
xattr -cr build/macos/Build/Products/Release/Rune.app

# Restore original project.pbxproj
mv macos/Runner.xcodeproj/project.pbxproj.backup macos/Runner.xcodeproj/project.pbxproj
