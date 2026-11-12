#!/usr/bin/env bash
# Over-the-air install via the tailnet: archive + export a development-signed IPA on the Mac, publish it
# with an itms-services manifest into $OTA_DIR (default ~/moonrise-ota), which a Tailscale container
# ($OTA_CONTAINER, default moonrise-ota) serves over HTTPS.
# Open https://<that container>.<tailnet>.ts.net on the iPhone (Tailscale VPN on) and tap Install.
set -euo pipefail
# fastlane is only on the old Mac.
ASKED=${MAC:-}; . "$(dirname "${BASH_SOURCE[0]}")/common.sh"; MAC=${ASKED:-${FASTLANE_MAC:-$MAC}}
no_dev scripts/ota.sh
need_mac
need_signing scripts/ota.sh
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT=${OTA_DIR:-$HOME/moonrise-ota}
# Monotonic build number, used both inside the app and in the manifest (iOS ignores mismatches/same versions).
BUILD=$(date +%Y%m%d%H%M)

rsync_retry -a --delete --exclude .git --exclude /private --exclude server/tools/local --exclude "build-*" --exclude Moonrise.xcodeproj --exclude "*.log" --exclude Vendor --exclude server/target "$DIR/" "$MAC:~/$MAC_DIR/"
ssh "$MAC" "$MAC_UNLOCK
$MAC_PATH; source $ASC_ENV
cd ~/$MAC_DIR && scripts/xcodegen.sh >/dev/null
rm -rf build-ota && mkdir -p build-ota
xcodebuild archive -project $APPLE_DIR/Moonrise.xcodeproj -scheme Moonrise -configuration Release -destination 'generic/platform=iOS' \
  -archivePath build-ota/Moonrise.xcarchive -derivedDataPath build-ota/dd -allowProvisioningUpdates -authenticationKeyID \$ASC_KEY_ID -authenticationKeyIssuerID \$ASC_ISSUER_ID -authenticationKeyPath $ASC_KEY_PATH CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=\$APPLE_TEAM_ID CURRENT_PROJECT_VERSION=$BUILD MOONRISE_BUILD_STAMP=$(build_stamp) \
  > build-ota/archive.log 2>&1 || { grep -E 'error' build-ota/archive.log | head -10; exit 1; }
(cd $APPLE_DIR && ASC_KEY_PATH=$ASC_KEY_PATH MOONRISE_BUNDLE_ID=$APP_ID fastlane adhoc_profiles) > build-ota/fastlane.log 2>&1 \
  || { grep -iE 'error|\\[!\\]' build-ota/fastlane.log | head -10; exit 1; }
cat > build-ota/export.plist <<PL
<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\"><dict>
<key>method</key><string>release-testing</string>
<key>teamID</key><string>\$APPLE_TEAM_ID</string>
<key>signingStyle</key><string>manual</string>
<key>signingCertificate</key><string>Apple Distribution</string>
<key>provisioningProfiles</key><dict>
  <key>$APP_ID</key><string>Moonrise AdHoc</string>
  <key>$APP_ID.widgets</key><string>Moonrise Widgets AdHoc</string>
</dict>
<key>compileBitcode</key><false/>
<key>thinning</key><string>&lt;none&gt;</string>
</dict></plist>
PL
xcodebuild -exportArchive -archivePath build-ota/Moonrise.xcarchive -exportPath build-ota/export -exportOptionsPlist build-ota/export.plist -allowProvisioningUpdates -authenticationKeyID \$ASC_KEY_ID -authenticationKeyIssuerID \$ASC_ISSUER_ID -authenticationKeyPath $ASC_KEY_PATH \
  > build-ota/export.log 2>&1 || { grep -E 'error' build-ota/export.log | head -10; exit 1; }
ls -la build-ota/export/*.ipa"

mkdir -p "$OUT"
scp -q "$MAC:~/$MAC_DIR/build-ota/export/Moonrise.ipa" "$OUT/Moonrise.ipa"
VERSION=$BUILD
DOMAIN=$(docker exec "${OTA_CONTAINER:-moonrise-ota}" tailscale status --json 2>/dev/null | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))' || echo "ota-host.example.ts.net")
cat > "$OUT/manifest.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>items</key><array><dict>
  <key>assets</key><array><dict><key>kind</key><string>software-package</string><key>url</key><string>https://$DOMAIN/Moonrise.ipa?v=$VERSION</string></dict></array>
  <key>metadata</key><dict>
    <key>bundle-identifier</key><string>$APP_ID</string>
    <key>bundle-version</key><string>$VERSION</string>
    <key>kind</key><string>software</string>
    <key>title</key><string>Moonrise</string>
  </dict>
</dict></array></dict></plist>
PL
cat > "$OUT/index.html" <<HTML
<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Moonrise</title>
<body style="background:#0d0f14;color:#fff;font:17px -apple-system;text-align:center;padding-top:30vh">
<h1>Moonrise</h1><p style="opacity:.6">Build $VERSION</p>
<a style="display:inline-block;margin-top:24px;padding:14px 40px;border-radius:30px;background:#fff;color:#000;text-decoration:none;font-weight:600"
   href="itms-services://?action=download-manifest&amp;url=https://$DOMAIN/manifest.plist">Install</a>
</body>
HTML
echo "Published build $VERSION → https://$DOMAIN/"
