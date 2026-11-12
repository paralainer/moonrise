#!/usr/bin/env bash
# Build signed for tvOS and install on the paired Apple TV (TV_DEVICE in mac.env) over the network.
# The default is the app itself; DEV=1 installs the Dev app ("Moonrise Dev", <bundle id>.dev) beside
# it, with its own sign-in and Top Shelf, for testing (scripts/common.sh).
#
# Checks first that the build Mac and the Apple TV can be reached, so an asleep TV fails before
# the build. Fails (exit 1) with devicectl's answer when the install doesn't happen; on success
# prints the build stamp and build number the TV now has (Settings shows the same stamp).
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
. "$DIR/scripts/common.sh"
need_mac
need_signing scripts/deploy-tv.sh
need "$TV_DEVICE" "no Apple TV paired with $MAC (TV_DEVICE in scripts/mac.env)"
need_device "$TV_DEVICE" "the Apple TV (TV_DEVICE)"
"$DIR/scripts/build.sh" tv
install_on_device "$TV_DEVICE" "the Apple TV" build-tv$BUILD_SUFFIX/Build/Products/Debug-appletvos/Moonrise.app
