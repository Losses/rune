#!/usr/bin/env sh

set -e

cd "$(dirname "$0")"
cd ..

rm -rf temp_macos
mkdir temp_macos

ditto build/macos/Build/Products/Release/Rune.app temp_macos/Rune.app
cp macos/Runner/Release.entitlements temp_macos
cp ~/Library/MobileDevice/Provisioning\ Profiles/*.provisionprofile temp_macos/Rune.app/Contents/embedded.provisionprofile

profile_name="$( ls -t ~/Library/MobileDevice/Provisioning\ Profiles/ | head -n1 )"
if [[ "$profile_name" == *"App_Store"* ]]; then
    sed -i '' 's/<string>LG57TUQ726.ci.not.rune<\/string>/<string>LG57TUQ726.ci.not.rune.appstore<\/string>/g' temp_macos/Release.entitlements
fi
