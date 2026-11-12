#!/usr/bin/env bash
# Build the Mac app signed and install it in /Applications on the build Mac (then open it).
# The default is the app itself (/Applications/Moonrise.app); DEV=1 installs the Dev app beside it
# (/Applications/Moonrise Dev.app, <bundle id>.dev, its own sign-in and data; scripts/common.sh).
# Only the app being installed is quit first; the other one keeps running.
#
# Fails (exit 1) when the Mac can't be reached, the build fails, or /Applications doesn't end up
# with the new build; on success prints the build stamp and build number installed.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$DIR/scripts/common.sh"
need_mac
need_signing scripts/deploy-mac.sh
need_mac_reachable
"$DIR/scripts/build.sh" mac
APP=build-mac$BUILD_SUFFIX/Build/Products/Debug/Moonrise.app
DEST="/Applications/$APP_NAME.app"
built=$(app_built "$APP" Contents/Info.plist) || { echo "error: no built app at $APP on $MAC" >&2; exit 1; }
echo "==> installing $APP_NAME ($APP_ID) ${built#* } (build ${built%% *}) in /Applications on $MAC"
# Quit this app only: the processes running the executable inside $DEST (both apps' executables are
# called Moonrise), by their pids.
ssh "$MAC" "pids=\$(pgrep -f '^$DEST/Contents/MacOS/Moonrise( |\$)' || true); [ -z \"\$pids\" ] || kill \$pids; rm -rf '$DEST' && ditto ~/$MAC_DIR/$APP '$DEST'" \
  || { echo "error: copying the app to $DEST on $MAC failed" >&2; exit 1; }
installed=$(ssh "$MAC" "/usr/libexec/PlistBuddy -c 'Print CFBundleVersion' '$DEST/Contents/Info.plist'" || true)
[ "$installed" = "${built%% *}" ] || { echo "error: $DEST on $MAC has build '${installed:-none}', not ${built%% *}" >&2; exit 1; }
ssh "$MAC" "open '$DEST'" || { echo "error: installed, but the app didn't open" >&2; exit 1; }
echo "==> $MAC now has $APP_NAME ${built#* } (build $installed) in $DEST, opened"
