#!/usr/bin/env bash
# Build signed for device and install on the iPhone over the network (IPAD=1: on the iPad).
# The default is the app itself; DEV=1 installs the Dev app ("Moonrise Dev", <bundle id>.dev) beside
# it, with its own sign-in, data and widgets, for testing (scripts/common.sh).
#
# Checks first that the build Mac and the device can be reached, so an asleep device fails before
# the build. Fails (exit 1) with devicectl's answer when the install doesn't happen; on success
# prints the build stamp and build number the device now has (Settings shows the same stamp).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$DIR/scripts/common.sh"
need_mac
need_signing scripts/deploy-phone.sh
if [ "${IPAD:-}" = 1 ]; then SETTING=IPAD_DEVICE; NAME="the iPad"; else SETTING=PHONE_DEVICE; NAME="the iPhone"; fi
DEVICE=${!SETTING}
need "$DEVICE" "no device: set $SETTING in scripts/mac.env (a device paired with $MAC: xcrun devicectl list devices)"
need_device "$DEVICE" "$NAME ($SETTING)"
"$DIR/scripts/build.sh" ios
install_on_device "$DEVICE" "$NAME" build-ios$BUILD_SUFFIX/Build/Products/Debug-iphoneos/Moonrise.app
