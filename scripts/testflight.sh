#!/usr/bin/env bash
# Store-signed build to TestFlight (internal "Team" group: every build, no review), through the Mac.
# Usage: scripts/testflight.sh [tvos|ios]   (default tvos). Needs the App Store Connect app record
# for the app's bundle id (website only); the first run also sets up the Team group.
set -euo pipefail
ASKED=${MAC:-}; . "$(dirname "${BASH_SOURCE[0]}")/common.sh"; MAC=${ASKED:-${FASTLANE_MAC:-$MAC}}
no_dev scripts/testflight.sh
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORM="${1:-tvos}"
case "$PLATFORM" in
  tvos) DEST="generic/platform=tvOS" ;;
  ios)  DEST="generic/platform=iOS" ;;
  *) echo "usage: $0 [tvos|ios]"; exit 2 ;;
esac
need_mac
need_signing scripts/testflight.sh
[ -f "$DIR/scripts/local.env" ] && . "$DIR/scripts/local.env"
BUILD=$(date +%Y%m%d%H%M)
rsync_retry -a --delete --exclude .git --exclude "build-*" --exclude Moonrise.xcodeproj --exclude "*.log" --exclude Vendor --exclude /private --exclude server/tools/local "$DIR/" "$MAC:~/$MAC_DIR/"
ssh "$MAC" "security unlock-keychain -p \"${MAC_KEYCHAIN_PASSWORD:-}\" ~/Library/Keychains/login.keychain-db >/dev/null 2>&1
set -e
export PATH=/usr/local/bin:\$PATH LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 FASTLANE_SKIP_UPDATE_CHECK=1; source $ASC_ENV
export ASC_KEY_PATH=$ASC_KEY_PATH MOONRISE_BUNDLE_ID=$APP_ID
AUTH=\"-allowProvisioningUpdates -authenticationKeyID \$ASC_KEY_ID -authenticationKeyIssuerID \$ASC_ISSUER_ID -authenticationKeyPath \$ASC_KEY_PATH\"
cd ~/$MAC_DIR && scripts/xcodegen.sh >/dev/null
(cd $APPLE_DIR && fastlane moonrise_app) 2>&1 | grep -o 'MOONRISE_APP=.*' | grep -qv none || { echo 'No App Store Connect app for $APP_ID yet (create it on the website)'; exit 1; }
OUT=build-tf-$PLATFORM; rm -rf \$OUT && mkdir -p \$OUT
eval xcodebuild archive -project $APPLE_DIR/Moonrise.xcodeproj -scheme Moonrise -configuration Release -destination '$DEST' \
  -archivePath \$OUT/Moonrise.xcarchive -derivedDataPath \$OUT/dd \$AUTH CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=\$APPLE_TEAM_ID \
  CURRENT_PROJECT_VERSION=$BUILD MOONRISE_BUILD_STAMP=$(build_stamp) > \$OUT/archive.log 2>&1 || { grep -E 'error' \$OUT/archive.log | sort -u | head -15; exit 1; }
cat > \$OUT/export.plist <<PL
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\"><dict>
<key>method</key><string>app-store-connect</string>
<key>destination</key><string>upload</string>
<key>teamID</key><string>\$APPLE_TEAM_ID</string>
<key>signingStyle</key><string>automatic</string>
<key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>
PL
eval xcodebuild -exportArchive -archivePath \$OUT/Moonrise.xcarchive -exportPath \$OUT/export -exportOptionsPlist \$OUT/export.plist \$AUTH \
  > \$OUT/export.log 2>&1 || { grep -iE 'error|fail' \$OUT/export.log | sort -u | head -15; exit 1; }
grep -iE 'Upload succeeded|uploaded|EXPORT SUCCEEDED' \$OUT/export.log | tail -2
(cd $APPLE_DIR && fastlane team_testers) 2>&1 | grep -E 'Created|Tester' || true"
echo "✓ $PLATFORM build $BUILD uploaded: TestFlight processes it in ~10-30 min, then it appears in the TestFlight app"
