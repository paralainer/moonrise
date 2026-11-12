#!/usr/bin/env bash
# Sync sources to the build Mac and compile. Usage: scripts/build.sh [ios|sim|tv|tvos|mac|macos]
# (default ios; sim, tvos = simulators, ad-hoc signed; mac = signed, for scripts/deploy-mac.sh;
# macos = unsigned compile check)
# Device builds are optimized (-O, still DEBUG): unoptimized SwiftUI bodies overflowed the iPad's
# 1 MB main-thread stack opening a title page.
# DEV=1 builds the Dev app ("Moonrise Dev", <bundle id>.dev; scripts/common.sh) into build-<platform>-dev.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PLATFORM="${1:-ios}"
case "$PLATFORM" in
  ios)   DEST="generic/platform=iOS" ;;
  sim)   DEST="generic/platform=iOS Simulator" ;;
  tvos)  DEST="generic/platform=tvOS Simulator" ;;
  tv)    DEST="generic/platform=tvOS" ;;   # signed, for the paired Apple TV (scripts/deploy-tv.sh)
  mac)   DEST="platform=macOS,arch=arm64" ;;
  macos) DEST="generic/platform=macOS" ;;
  *) echo "usage: $0 [ios|sim|tv|tvos|mac|macos]" >&2; exit 2 ;;
esac
need_mac
# Signed builds (ios, tv, mac) use your team and the App Store Connect key in ASC_ENV on the Mac;
# the others need neither.
case "$PLATFORM" in
  ios|tv|mac)
    need_signing "scripts/build.sh $PLATFORM"
    ASC="source $ASC_ENV"
    SIGN="-allowProvisioningUpdates SWIFT_OPTIMIZATION_LEVEL=-O -authenticationKeyID \$ASC_KEY_ID -authenticationKeyIssuerID \$ASC_ISSUER_ID -authenticationKeyPath $ASC_KEY_PATH CODE_SIGN_STYLE=Automatic DEVELOPMENT_TEAM=\$APPLE_TEAM_ID" ;;
  # Simulators: ad-hoc signed, so the entitlements apply (background URLSession on iOS; the keychain
  # group the Apple TV app shares with its Top Shelf).
  sim|tvos) ASC=:; SIGN="CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM= PROVISIONING_PROFILE_SPECIFIER=" ;;
  *)   ASC=:; SIGN="CODE_SIGNING_ALLOWED=NO" ;;
esac
# The build stamp (commit) and a build number unique to this build, so the deploy scripts can tell
# the new build from the one on the device; Settings shows the stamp next to the version.
STAMP=$(build_stamp)
STAMPS="MOONRISE_BUILD_STAMP=$STAMP CURRENT_PROJECT_VERSION=$(date +%Y%m%d%H%M%S)"
OUT=build-$PLATFORM$BUILD_SUFFIX
echo "==> building $PLATFORM: $APP_NAME ($APP_ID) $STAMP"
# The copy is tried again when its connection fails. The build itself is not: it could
# already be running on the Mac when the connection drops, so it gets a fresh reachability check
# (with retries) and then one ssh.
rsync_retry -a --delete --exclude .git --exclude /private --exclude "server/tools/local" --exclude "build-*" --exclude Moonrise.xcodeproj --exclude "*.log" --exclude Vendor --exclude server/target "$DIR/" "$MAC:~/$MAC_DIR/"
need_mac_reachable
ssh "${MAC_SSH_OPTS[@]}" "$MAC" "$MAC_UNLOCK
$MAC_PATH; $ASC
cd ~/$MAC_DIR && scripts/xcodegen.sh >/dev/null
xcodebuild build -project $APPLE_DIR/Moonrise.xcodeproj -scheme Moonrise -configuration Debug \
  -destination '$DEST' -derivedDataPath $OUT $SIGN $STAMPS $VARIANT_SETTINGS \
  > $OUT.log 2>&1; rc=\$?
grep -E '(error|warning: unre).*:' $OUT.log | grep -v '^ld: warning' | sort -u | head -60
grep -E '^\\*\\* BUILD' $OUT.log; exit \$rc"
