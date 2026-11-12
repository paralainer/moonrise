#!/usr/bin/env bash
# Screenshot the Apple TV simulator (1280 wide). Usage: scripts/tvshot.sh name
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/common.sh"
need_mac
need "$SIM_TV" "no Apple TV simulator: set SIM_TV in scripts/mac.env (a UDID from xcrun simctl list devices on the Mac; scripts/sim-create.sh tv makes one)"
OUT=${SHOT_DIR:-/tmp/moonrise-shots}
mkdir -p "$OUT"
ssh "$MAC" "xcrun simctl io $SIM_TV screenshot /tmp/moonrise-tv.png >/dev/null 2>&1 && sips -Z 1280 /tmp/moonrise-tv.png >/dev/null"
scp -q "$MAC":/tmp/moonrise-tv.png "$OUT/$1.png"
echo "$OUT/$1.png"
